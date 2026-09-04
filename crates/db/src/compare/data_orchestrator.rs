use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use gpui::AsyncApp;
use one_core::gpui_tokio::Tokio;
use one_core::storage::DatabaseType;

use crate::{
    ColumnInfo, ForeignKeyDefinition, GlobalDbState, QueryColumnMeta, QueryResult, SqlResult,
    TableDataResponse, TableInfo, TableObjectType, plugin::DatabasePlugin,
};

use super::data_paging::{apply_keyset_where_clause, build_keyset_where_clause};
use super::{
    CompareRowSide, CompareTaskEvent, DEFAULT_DATA_COMPARE_PAGE_SIZE, DataCompareBatchResult,
    DataCompareBatchWarning, DataCompareBatchWarningKind, DataCompareColumnMapping,
    DataCompareLimits, DataCompareOptions, DataComparePagingDecision, DataCompareResult,
    DataCompareTableDependency, DataCompareTableFailure, DataCompareTablePair, RowData, SyncPlan,
    TableSchema, append_table_data_page, build_table_data_request, common_column_mappings,
    compare_data_rows, data_compare_next_page_size, data_compare_paging_decision, identifier_key,
    map_column_type, normalize_compare_table_data_response, rows_from_query_result_with_mappings,
    table_data_terminal_probe_required, table_schema_from_columns,
};

/// Records one table result without allowing a single table failure to discard
/// results that were already compared successfully.
pub fn record_data_compare_pair_result(
    table_results: &mut Vec<DataCompareResult>,
    table_failures: &mut Vec<DataCompareTableFailure>,
    target_table: String,
    result: anyhow::Result<DataCompareResult>,
) {
    match result {
        Ok(result) => table_results.push(result),
        Err(error) => table_failures.push(DataCompareTableFailure {
            table: target_table,
            error: format!("{error:#}"),
        }),
    }
}

impl GlobalDbState {
    /// Builds the data synchronization plan using the target connection's
    /// dialect. A blocked result intentionally falls back to the generic plan
    /// so callers can still inspect warnings without requiring a live target
    /// connection or plugin.
    pub fn prepare_data_sync_plan_for_target(
        &self,
        result: &DataCompareBatchResult,
        target_connection_id: &str,
        target_database: &str,
        target_schema: Option<&str>,
    ) -> anyhow::Result<SyncPlan> {
        if result.is_sync_sql_blocked() {
            return Ok(super::build_data_sync_batch_plan(result));
        }

        let config = self
            .get_config(target_connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {target_connection_id}"))?;
        let plugin = self.get_plugin(&config.database_type)?;

        Ok(super::build_data_sync_batch_plan_with_plugin(
            result,
            target_database,
            target_schema,
            plugin.as_ref(),
        ))
    }

    /// Loads, compares, and prepares all selected table pairs.
    ///
    /// This is deliberately database-layer orchestration. Callers only provide
    /// connection/table parameters and translate the structured events into
    /// localized progress UI.
    pub async fn prepare_data_compare_from_tables(
        &self,
        cx: &mut AsyncApp,
        params: super::DataCompareParams,
        mut report: impl FnMut(CompareTaskEvent),
    ) -> anyhow::Result<DataCompareBatchResult> {
        let started_at = Instant::now();
        let total_tables = params.table_pairs.len();
        report(CompareTaskEvent::Started {
            task_id: uuid::Uuid::new_v4().to_string(),
            total_tables,
        });

        let mut table_results = Vec::with_capacity(total_tables);
        let mut table_failures = Vec::new();
        let mut batch_warnings = data_compare_snapshot_warnings(self, &params)?;
        for (index, pair) in params.table_pairs.iter().cloned().enumerate() {
            report(CompareTaskEvent::TableStarted {
                table: pair.target_table.clone(),
                table_index: index + 1,
                total_tables,
            });
            let target_table = pair.target_table.clone();
            let result = execute_data_compare_pair(self, cx, &params, pair, &mut report).await;
            match &result {
                Ok(result) => {
                    report(CompareTaskEvent::TableFinished {
                        table: result.target_table.clone(),
                        added: result.added.len(),
                        removed: result.removed.len(),
                        modified: result.modified.len(),
                    });
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    report(CompareTaskEvent::Error {
                        table: Some(target_table.clone()),
                        message: message.clone(),
                    });
                }
            }
            record_data_compare_pair_result(
                &mut table_results,
                &mut table_failures,
                target_table,
                result,
            );
        }

        report(CompareTaskEvent::LoadingMetadata { table: None });
        let successful_target_tables =
            successful_target_table_keys(&table_results, params.case_sensitive_identifiers);
        let missing_target_tables =
            missing_target_table_keys(&table_results, params.case_sensitive_identifiers);
        let dependency_result = if successful_target_tables.is_empty() {
            DataCompareDependencyLoadResult::default()
        } else {
            load_data_compare_table_dependencies(
                self,
                cx,
                &params,
                &successful_target_tables,
                &missing_target_tables,
                &mut report,
            )
            .await
        };
        let result = DataCompareBatchResult {
            table_results,
            table_dependencies: dependency_result.dependencies,
            table_failures,
            batch_warnings: {
                batch_warnings.extend(dependency_result.warnings);
                batch_warnings
            },
        };
        report(CompareTaskEvent::Finished {
            elapsed_ms: started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
        });
        Ok(result)
    }
}

async fn execute_data_compare_pair(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    params: &super::DataCompareParams,
    pair: DataCompareTablePair,
    report: &mut impl FnMut(CompareTaskEvent),
) -> anyhow::Result<DataCompareResult> {
    let source_table = target_table_info(
        db_state,
        cx,
        &params.source_connection_id,
        &params.source_database,
        params.source_schema.clone(),
        &pair.source_table,
        params.case_sensitive_identifiers,
    )
    .await?;
    if matches!(
        source_table.as_ref().map(|table| table.object_type),
        Some(TableObjectType::View)
    ) {
        anyhow::bail!(
            "Source object `{}` is a view; data comparison and synchronization for views is not supported",
            pair.source_table
        );
    }

    report(CompareTaskEvent::LoadingMetadata {
        table: Some(pair.source_table.clone()),
    });
    let source_columns = load_table_columns(
        db_state,
        cx,
        &params.source_connection_id,
        &params.source_database,
        params.source_schema.clone(),
        &pair.source_table,
    )
    .await?;

    let target_table = target_table_info(
        db_state,
        cx,
        &params.target_connection_id,
        &params.target_database,
        params.target_schema.clone(),
        &pair.target_table,
        params.case_sensitive_identifiers,
    )
    .await?;
    match target_table {
        None => {
            return execute_data_compare_missing_target_pair(
                db_state,
                cx,
                params,
                pair,
                source_columns,
                report,
            )
            .await;
        }
        Some(target_table) if target_table.object_type == TableObjectType::View => {
            anyhow::bail!(
                "Target object `{}` is a view; data comparison and synchronization for views is not supported",
                pair.target_table
            );
        }
        Some(_) => {}
    }

    report(CompareTaskEvent::LoadingMetadata {
        table: Some(pair.target_table.clone()),
    });
    let target_columns = load_table_columns(
        db_state,
        cx,
        &params.target_connection_id,
        &params.target_database,
        params.target_schema.clone(),
        &pair.target_table,
    )
    .await?;
    let key_columns = resolve_key_columns_for_table(
        &params.key_columns,
        &source_columns,
        &target_columns,
        params.case_sensitive_identifiers,
        &pair,
    )?;
    let target_key_columns = matching_target_columns(
        &key_columns,
        &target_columns,
        params.case_sensitive_identifiers,
    );

    report(CompareTaskEvent::CountingRows {
        table: pair.source_table.clone(),
        side: CompareRowSide::Source,
    });
    let source_response = load_table_data(
        db_state,
        cx,
        &params.source_connection_id,
        &params.source_database,
        params.source_schema.clone(),
        &pair.source_table,
        &key_columns,
        &source_columns,
        params.case_sensitive_identifiers,
        params.limits,
        CompareRowSide::Source,
        &pair.source_table,
        report,
    )
    .await?;
    report(CompareTaskEvent::CountingRows {
        table: pair.target_table.clone(),
        side: CompareRowSide::Target,
    });
    let target_response = load_table_data(
        db_state,
        cx,
        &params.target_connection_id,
        &params.target_database,
        params.target_schema.clone(),
        &pair.target_table,
        &target_key_columns,
        &target_columns,
        params.case_sensitive_identifiers,
        params.limits,
        CompareRowSide::Target,
        &pair.target_table,
        report,
    )
    .await?;

    report(CompareTaskEvent::ComparingRows {
        table: pair.target_table.clone(),
    });
    build_data_compare_result(
        pair,
        key_columns,
        source_response,
        target_response,
        params.case_sensitive_identifiers,
    )
}

async fn execute_data_compare_missing_target_pair(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    params: &super::DataCompareParams,
    pair: DataCompareTablePair,
    source_columns: Vec<ColumnInfo>,
    report: &mut impl FnMut(CompareTaskEvent),
) -> anyhow::Result<DataCompareResult> {
    let source_db_type = db_state
        .get_config(&params.source_connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", params.source_connection_id))?
        .database_type
        .clone();
    let target_db_type = db_state
        .get_config(&params.target_connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", params.target_connection_id))?
        .database_type
        .clone();
    let target_columns =
        map_missing_target_columns(&source_columns, &source_db_type, &target_db_type)?;
    let key_columns = resolve_key_columns_for_table(
        &params.key_columns,
        &source_columns,
        &source_columns,
        params.case_sensitive_identifiers,
        &pair,
    )?;
    report(CompareTaskEvent::CountingRows {
        table: pair.source_table.clone(),
        side: CompareRowSide::Source,
    });
    let source_response = load_table_data(
        db_state,
        cx,
        &params.source_connection_id,
        &params.source_database,
        params.source_schema.clone(),
        &pair.source_table,
        &key_columns,
        &source_columns,
        params.case_sensitive_identifiers,
        params.limits,
        CompareRowSide::Source,
        &pair.source_table,
        report,
    )
    .await?;
    report(CompareTaskEvent::ComparingRows {
        table: pair.target_table.clone(),
    });
    build_missing_target_table_result(
        pair,
        key_columns,
        &target_columns,
        source_response,
        params.case_sensitive_identifiers,
    )
}

fn map_missing_target_columns(
    source_columns: &[ColumnInfo],
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
) -> anyhow::Result<Vec<ColumnInfo>> {
    source_columns
        .iter()
        .map(|column| {
            let mapping = map_column_type(&column.data_type, source_db_type, target_db_type);
            if !mapping.compatibility.is_safe_for_automatic_sync() {
                let warning = mapping.warning.as_deref().unwrap_or(
                    "目标数据库无法在不损失字段语义或精度的情况下表示该类型",
                );
                anyhow::bail!(
                    "字段 `{}` 的类型 `{}` 无法安全映射到目标数据库类型 `{}`，无法为缺失目标表生成 CREATE TABLE：{}",
                    column.name,
                    column.data_type,
                    mapping.target_type,
                    warning
                );
            }
            let mut mapped = column.clone();
            mapped.data_type = mapping.target_type;
            Ok(mapped)
        })
        .collect()
}

async fn load_table_columns(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    table: &str,
) -> anyhow::Result<Vec<ColumnInfo>> {
    let db_state = db_state.clone();
    let connection_id = connection_id.to_string();
    let database = database.to_string();
    let table = table.to_string();
    Tokio::spawn_result(cx, async move {
        db_state
            .list_columns_direct(&connection_id, &database, schema, &table)
            .await
    })
    .await
}

async fn target_table_info(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    table: &str,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<Option<TableInfo>> {
    let db_state = db_state.clone();
    let connection_id = connection_id.to_string();
    let database = database.to_string();
    let tables = Tokio::spawn_result(cx, async move {
        db_state
            .list_tables_direct(&connection_id, &database, schema)
            .await
    })
    .await?;
    Ok(matching_table(&tables, table, case_sensitive_identifiers).cloned())
}

fn matching_table<'a>(
    tables: &'a [TableInfo],
    table: &str,
    case_sensitive_identifiers: bool,
) -> Option<&'a TableInfo> {
    let expected = table_lookup_key(table, case_sensitive_identifiers);
    tables
        .iter()
        .find(|candidate| table_lookup_key(&candidate.name, case_sensitive_identifiers) == expected)
}

async fn load_table_data(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    table: &str,
    key_columns: &[String],
    business_columns: &[ColumnInfo],
    case_sensitive_identifiers: bool,
    limits: DataCompareLimits,
    side: CompareRowSide,
    progress_table: &str,
    report: &mut impl FnMut(CompareTaskEvent),
) -> anyhow::Result<TableDataResponse> {
    let config = db_state
        .get_config(connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {connection_id}"))?;
    let snapshot = data_compare_snapshot_strategy(&config.database_type);
    let DataCompareSnapshotStrategy::Transaction(begin_statements) = snapshot else {
        return load_table_data_pages(
            db_state,
            cx,
            connection_id,
            None,
            database,
            schema,
            table,
            key_columns,
            business_columns,
            case_sensitive_identifiers,
            limits,
            side,
            progress_table,
            report,
        )
        .await;
    };

    let session_id = db_state
        .create_session(
            cx,
            connection_id.to_string(),
            Some(database.to_string()),
        )
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to open a pinned session for the consistent read snapshot on {side:?}: {error:#}"
            )
        })?;

    if let Err(error) =
        begin_data_compare_snapshot(db_state, cx, &session_id, begin_statements).await
    {
        let cleanup_error = cleanup_data_compare_snapshot(db_state, cx, &session_id)
            .await
            .err();
        return Err(attach_snapshot_cleanup_error(error, cleanup_error));
    }

    let result = load_table_data_pages(
        db_state,
        cx,
        connection_id,
        Some(&session_id),
        database,
        schema,
        table,
        key_columns,
        business_columns,
        case_sensitive_identifiers,
        limits,
        side,
        progress_table,
        report,
    )
    .await;
    let cleanup_error = cleanup_data_compare_snapshot(db_state, cx, &session_id)
        .await
        .err();
    match result {
        Ok(response) => match cleanup_error {
            Some(error) => Err(error),
            None => Ok(response),
        },
        Err(error) => Err(attach_snapshot_cleanup_error(error, cleanup_error)),
    }
}

async fn load_table_data_pages(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    session_id: Option<&str>,
    database: &str,
    schema: Option<String>,
    table: &str,
    key_columns: &[String],
    business_columns: &[ColumnInfo],
    case_sensitive_identifiers: bool,
    limits: DataCompareLimits,
    side: CompareRowSide,
    progress_table: &str,
    report: &mut impl FnMut(CompareTaskEvent),
) -> anyhow::Result<TableDataResponse> {
    let config = db_state
        .get_config(connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {connection_id}"))?;
    let plugin = db_state.get_plugin(&config.database_type)?;
    let order_by_clause = quoted_order_by_clause(plugin.as_ref(), key_columns);
    let has_internal_rowid = plugin.supports_rowid();
    let internal_rowid_alias = plugin.rowid_column_alias().to_string();
    let mut page = 1usize;
    let mut accumulated: Option<TableDataResponse> = None;
    let mut keyset_where_clause: Option<String> = None;

    loop {
        let accumulated_rows = accumulated
            .as_ref()
            .map(|response| response.query_result.rows.len())
            .unwrap_or_default();
        let requested_page_size =
            data_compare_next_page_size(DEFAULT_DATA_COMPARE_PAGE_SIZE, accumulated_rows, limits)?;
        let request = build_table_data_request(
            database.to_string(),
            schema.clone(),
            table.to_string(),
            order_by_clause.as_deref(),
            page,
            accumulated_rows,
            requested_page_size,
        );
        let mut request = apply_keyset_where_clause(request, keyset_where_clause.as_deref());
        if let Some(total_count) = accumulated.as_ref().map(|response| response.total_count) {
            request = request.with_known_total_count(total_count);
        }
        let response =
            query_compare_table_data(db_state, cx, connection_id, session_id, request).await?;
        let response = normalize_compare_table_data_response(
            response,
            has_internal_rowid.then_some(internal_rowid_alias.as_str()),
            &config.database_type,
            business_columns,
        )?;
        let page_row_count = response.query_result.rows.len();
        let next_keyset_where_clause = build_keyset_where_clause(
            plugin.as_ref(),
            key_columns,
            business_columns,
            &response.query_result,
            case_sensitive_identifiers,
        )?;
        append_table_data_page(&mut accumulated, response)?;
        keyset_where_clause = next_keyset_where_clause;
        let accumulated_response = accumulated
            .as_ref()
            .expect("append_table_data_page always initializes the accumulator");
        report(CompareTaskEvent::FetchingRows {
            table: progress_table.to_string(),
            side,
            fetched_rows: accumulated_response.query_result.rows.len(),
            total_rows: Some(accumulated_response.total_count),
        });
        match data_compare_paging_decision(
            accumulated_response.query_result.rows.len(),
            accumulated_response.total_count,
            page_row_count,
            requested_page_size,
            page,
            limits,
        )? {
            DataComparePagingDecision::Complete => {
                if table_data_terminal_probe_required(
                    accumulated_response.query_result.rows.len(),
                    accumulated_response.total_count,
                    page_row_count,
                    requested_page_size,
                ) {
                    let probe_page = page
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("data compare page number overflow"))?;
                    let probe_request = build_table_data_request(
                        database.to_string(),
                        schema.clone(),
                        table.to_string(),
                        order_by_clause.as_deref(),
                        probe_page,
                        accumulated_response.query_result.rows.len(),
                        1,
                    )
                    .with_known_total_count(accumulated_response.total_count);
                    let probe_request =
                        apply_keyset_where_clause(probe_request, keyset_where_clause.as_deref());
                    let probe = query_compare_table_data(
                        db_state,
                        cx,
                        connection_id,
                        session_id,
                        probe_request,
                    )
                    .await?;
                    let probe = normalize_compare_table_data_response(
                        probe,
                        has_internal_rowid.then_some(internal_rowid_alias.as_str()),
                        &config.database_type,
                        business_columns,
                    )?;
                    append_table_data_page(&mut accumulated, probe)?;
                }
                return Ok(accumulated.take().expect("accumulated response exists"));
            }
            DataComparePagingDecision::Truncated => {
                return Ok(accumulated.take().expect("accumulated response exists"));
            }
            DataComparePagingDecision::Continue => {}
        }
        page = page
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("data compare page number overflow"))?;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataCompareSnapshotStrategy {
    Transaction(&'static [&'static str]),
    BestEffort(&'static str),
}

fn data_compare_snapshot_strategy(database_type: &DatabaseType) -> DataCompareSnapshotStrategy {
    match database_type {
        DatabaseType::PostgreSQL => DataCompareSnapshotStrategy::Transaction(&[
            "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY",
        ]),
        DatabaseType::MySQL => DataCompareSnapshotStrategy::Transaction(&[
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            "START TRANSACTION READ ONLY",
        ]),
        DatabaseType::SQLite => DataCompareSnapshotStrategy::Transaction(&["BEGIN"]),
        DatabaseType::MSSQL => DataCompareSnapshotStrategy::BestEffort(
            "SQL Server SNAPSHOT isolation requires database-level ALLOW_SNAPSHOT_ISOLATION configuration; the compare workflow does not silently substitute a long-running SERIALIZABLE transaction",
        ),
        DatabaseType::ClickHouse => DataCompareSnapshotStrategy::BestEffort(
            "ClickHouse does not provide a transaction snapshot spanning the separate COUNT and page queries used by this workflow",
        ),
        DatabaseType::TDengine => DataCompareSnapshotStrategy::BestEffort(
            "TDengine does not provide a transaction snapshot spanning the separate COUNT and page queries used by this workflow",
        ),
        DatabaseType::DuckDB => DataCompareSnapshotStrategy::BestEffort(
            "DuckDB snapshot semantics have not been enabled for the compare workflow",
        ),
        DatabaseType::Oracle => DataCompareSnapshotStrategy::BestEffort(
            "Oracle snapshot semantics have not been enabled for the compare workflow",
        ),
        DatabaseType::External { .. } => DataCompareSnapshotStrategy::BestEffort(
            "external drivers do not declare a host-verifiable consistent snapshot contract",
        ),
    }
}

fn data_compare_snapshot_warnings(
    db_state: &GlobalDbState,
    params: &super::DataCompareParams,
) -> anyhow::Result<Vec<DataCompareBatchWarning>> {
    let sides = [
        (
            "source",
            params.source_connection_id.as_str(),
            params.source_database.as_str(),
        ),
        (
            "target",
            params.target_connection_id.as_str(),
            params.target_database.as_str(),
        ),
    ];
    let mut warnings = Vec::new();
    for (side, connection_id, database) in sides {
        let config = db_state
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {connection_id}"))?;
        if let DataCompareSnapshotStrategy::BestEffort(reason) =
            data_compare_snapshot_strategy(&config.database_type)
        {
            warnings.push(DataCompareBatchWarning {
                table: None,
                kind: DataCompareBatchWarningKind::ConsistentSnapshotUnavailable,
                error: format!(
                    "{side} connection `{connection_id}` database `{database}` ({}) is using best-effort paging: {reason}",
                    config.database_type.as_str()
                ),
            });
        }
    }
    Ok(warnings)
}

async fn query_compare_table_data(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    session_id: Option<&str>,
    request: crate::types::TableDataRequest,
) -> anyhow::Result<TableDataResponse> {
    match session_id {
        Some(session_id) => {
            db_state
                .query_table_data_session_on_runtime(cx, session_id.to_string(), request)
                .await
        }
        None => {
            db_state
                .query_table_data(cx, connection_id.to_string(), request)
                .await
        }
    }
}

async fn begin_data_compare_snapshot(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    session_id: &str,
    statements: &[&str],
) -> anyhow::Result<()> {
    for statement in statements {
        execute_snapshot_statement(db_state, cx, session_id, statement)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to begin consistent read snapshot with `{statement}`: {error:#}"
                )
            })?;
    }
    Ok(())
}

async fn cleanup_data_compare_snapshot(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    session_id: &str,
) -> anyhow::Result<()> {
    let rollback_error = execute_snapshot_statement(db_state, cx, session_id, "ROLLBACK")
        .await
        .err();
    let close_error = db_state
        .close_session(cx, session_id.to_string())
        .await
        .err();
    match (rollback_error, close_error) {
        (None, None) => Ok(()),
        (Some(error), None) => Err(anyhow::anyhow!(
            "failed to roll back consistent read snapshot: {error:#}"
        )),
        (None, Some(error)) => Err(anyhow::anyhow!(
            "failed to close consistent read snapshot session: {error:#}"
        )),
        (Some(rollback), Some(close)) => Err(anyhow::anyhow!(
            "failed to roll back consistent read snapshot: {rollback:#}; failed to close its session: {close:#}"
        )),
    }
}

async fn execute_snapshot_statement(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    session_id: &str,
    statement: &str,
) -> anyhow::Result<()> {
    let results = db_state
        .execute_session_on_runtime(cx, session_id.to_string(), statement.to_string(), None)
        .await?;
    if results.is_empty() {
        anyhow::bail!("database returned no result");
    }
    for result in results {
        if let SqlResult::Error(error) = result {
            anyhow::bail!("{}", error.message);
        }
    }
    Ok(())
}

fn attach_snapshot_cleanup_error(
    error: anyhow::Error,
    cleanup_error: Option<anyhow::Error>,
) -> anyhow::Error {
    match cleanup_error {
        Some(cleanup_error) => error.context(format!(
            "consistent read snapshot cleanup also failed: {cleanup_error:#}"
        )),
        None => error,
    }
}

#[derive(Debug, Default)]
struct DataCompareDependencyLoadResult {
    dependencies: Vec<DataCompareTableDependency>,
    warnings: Vec<DataCompareBatchWarning>,
}

#[derive(Clone, Copy)]
struct DataCompareDependencyScope<'a> {
    source_database: &'a str,
    source_schema: Option<&'a str>,
    target_database: &'a str,
    target_schema: Option<&'a str>,
    case_sensitive_identifiers: bool,
}

async fn load_data_compare_table_dependencies(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    params: &super::DataCompareParams,
    successful_target_tables: &HashSet<String>,
    missing_target_tables: &HashSet<String>,
    report: &mut impl FnMut(CompareTaskEvent),
) -> DataCompareDependencyLoadResult {
    let mut result = DataCompareDependencyLoadResult::default();
    report(CompareTaskEvent::LoadingDependencyMetadata { table: None });
    let existing_target_tables = match load_target_table_lookup(db_state, cx, params).await {
        Ok(tables) => tables,
        Err(error) => {
            record_dependency_metadata_failure(
                &mut result.warnings,
                None,
                DataCompareBatchWarningKind::TargetTableMetadataUnavailable,
                error,
            );
            return result;
        }
    };
    let selected_target_tables = target_table_lookup(params);
    let mut seen = HashSet::new();
    for pair in &params.table_pairs {
        let table_key = table_lookup_key(&pair.target_table, params.case_sensitive_identifiers);
        let successful = successful_target_tables.contains(&table_key);
        let existing_name = existing_target_tables.get(&table_key);
        if !successful {
            continue;
        }
        if existing_name.is_none() {
            if !missing_target_tables.contains(&table_key) {
                record_dependency_metadata_failure(
                    &mut result.warnings,
                    Some(pair.target_table.clone()),
                    DataCompareBatchWarningKind::TargetTableMetadataUnavailable,
                    anyhow::anyhow!(
                        "target table `{}` disappeared before dependency metadata was loaded",
                        pair.target_table
                    ),
                );
            }
            continue;
        }
        report(CompareTaskEvent::LoadingDependencyMetadata {
            table: Some(pair.target_table.clone()),
        });
        let foreign_keys =
            match load_target_foreign_keys(db_state, cx, params, &pair.target_table).await {
                Ok(foreign_keys) => foreign_keys,
                Err(error) => {
                    record_dependency_metadata_failure(
                        &mut result.warnings,
                        Some(pair.target_table.clone()),
                        DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable,
                        error,
                    );
                    continue;
                }
            };
        collect_table_dependencies(
            &mut result.dependencies,
            &mut seen,
            &selected_target_tables,
            &pair.target_table,
            foreign_keys,
            DataCompareDependencyScope {
                source_database: params.source_database.as_str(),
                source_schema: params.source_schema.as_deref(),
                target_database: params.target_database.as_str(),
                target_schema: params.target_schema.as_deref(),
                case_sensitive_identifiers: params.case_sensitive_identifiers,
            },
        );
    }
    result
}

pub fn record_dependency_metadata_failure(
    warnings: &mut Vec<DataCompareBatchWarning>,
    table: Option<String>,
    kind: DataCompareBatchWarningKind,
    error: anyhow::Error,
) {
    warnings.push(DataCompareBatchWarning {
        table,
        kind,
        error: format!("{error:#}"),
    });
}

async fn load_target_table_lookup(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    params: &super::DataCompareParams,
) -> anyhow::Result<HashMap<String, String>> {
    let db_state = db_state.clone();
    let connection_id = params.target_connection_id.clone();
    let database = params.target_database.clone();
    let schema = params.target_schema.clone();
    let tables = Tokio::spawn_result(cx, async move {
        db_state
            .list_tables_direct(&connection_id, &database, schema)
            .await
    })
    .await?;
    Ok(tables
        .into_iter()
        .map(|table| {
            (
                table_lookup_key(&table.name, params.case_sensitive_identifiers),
                table.name,
            )
        })
        .collect())
}

async fn load_target_foreign_keys(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    params: &super::DataCompareParams,
    table: &str,
) -> anyhow::Result<Vec<ForeignKeyDefinition>> {
    let db_state = db_state.clone();
    let connection_id = params.target_connection_id.clone();
    let database = params.target_database.clone();
    let schema = params.target_schema.clone();
    let table = table.to_string();
    Tokio::spawn_result(cx, async move {
        db_state
            .list_foreign_keys_direct(&connection_id, &database, schema, &table)
            .await
    })
    .await
}

fn target_table_lookup(params: &super::DataCompareParams) -> HashMap<String, String> {
    params
        .table_pairs
        .iter()
        .map(|pair| {
            (
                table_lookup_key(&pair.target_table, params.case_sensitive_identifiers),
                pair.target_table.clone(),
            )
        })
        .collect()
}

fn identifiers_equal(left: &str, right: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        left.trim() == right.trim()
    } else {
        left.trim().eq_ignore_ascii_case(right.trim())
    }
}

fn collect_table_dependencies(
    dependencies: &mut Vec<DataCompareTableDependency>,
    seen: &mut HashSet<(String, String)>,
    target_tables: &HashMap<String, String>,
    table: &str,
    foreign_keys: Vec<ForeignKeyDefinition>,
    scope: DataCompareDependencyScope<'_>,
) {
    for foreign_key in foreign_keys {
        let (parent_name, _, _) = resolve_dependency_parent_name(&foreign_key, scope);
        let parent_key = table_lookup_key(&parent_name, scope.case_sensitive_identifiers);
        let parent_table = target_tables
            .get(&parent_key)
            .cloned()
            .unwrap_or(parent_name);
        if table_lookup_key(table, scope.case_sensitive_identifiers) == parent_key {
            continue;
        }
        let edge = (table.to_string(), parent_table);
        if seen.insert(edge.clone()) {
            dependencies.push(DataCompareTableDependency {
                table: edge.0,
                referenced_table: edge.1,
            });
        }
    }
}

fn resolve_dependency_parent_name(
    foreign_key: &ForeignKeyDefinition,
    scope: DataCompareDependencyScope<'_>,
) -> (String, bool, bool) {
    let Some(namespace) = foreign_key.ref_schema.as_deref() else {
        return (foreign_key.ref_table.clone(), false, true);
    };
    let source_matches = namespace_matches_scope(
        namespace,
        scope.source_database,
        scope.source_schema,
        scope.case_sensitive_identifiers,
    );
    let target_matches = namespace_matches_scope(
        namespace,
        scope.target_database,
        scope.target_schema,
        scope.case_sensitive_identifiers,
    );
    let parent_name = if source_matches || target_matches {
        foreign_key.ref_table.clone()
    } else {
        format!("{namespace}.{}", foreign_key.ref_table)
    };
    (parent_name, source_matches, target_matches)
}

fn namespace_matches_scope(
    namespace: &str,
    database: &str,
    schema: Option<&str>,
    case_sensitive: bool,
) -> bool {
    identifiers_equal(namespace, database, case_sensitive)
        || schema.is_some_and(|schema| identifiers_equal(namespace, schema, case_sensitive))
}

pub fn build_data_compare_result(
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
    let source_mappings = columns
        .iter()
        .map(|column| DataCompareColumnMapping {
            source: column.source.clone(),
            target: column.source.clone(),
        })
        .collect::<Vec<_>>();
    let source_rows = rows_from_query_result_with_mappings(
        &source_response.query_result,
        &source_mappings,
        case_sensitive_identifiers,
    )
    .map_err(|error| anyhow::anyhow!("Invalid source comparison data: {error}"))?;
    let target_rows = rows_from_query_result_with_mappings(
        &target_response.query_result,
        &columns,
        case_sensitive_identifiers,
    )
    .map_err(|error| anyhow::anyhow!("Invalid target comparison data: {error}"))?;
    let mut result = compare_data_rows(
        source_rows,
        target_rows,
        DataCompareOptions {
            source_table: pair.source_table,
            target_table: pair.target_table,
            key_columns,
            columns: compare_columns,
        },
    )?;
    result.source_truncated = source_response.query_result.rows.len() < source_response.total_count;
    result.target_truncated = target_response.query_result.rows.len() < target_response.total_count;
    let result = remap_data_compare_result_to_target_columns(result, &columns);
    Ok(attach_target_column_types(
        result,
        &target_response.query_result.column_meta,
    ))
}

pub fn build_missing_target_table_result(
    pair: DataCompareTablePair,
    key_columns: Vec<String>,
    source_columns: &[ColumnInfo],
    source_response: TableDataResponse,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<DataCompareResult> {
    let mut result = build_data_compare_result(
        pair.clone(),
        key_columns,
        source_response,
        empty_target_response_from_columns(source_columns),
        case_sensitive_identifiers,
    )?;
    result.target_table_missing = true;
    result.missing_target_schema = Some(missing_target_table_schema(
        &pair.target_table,
        source_columns,
    ));
    Ok(result)
}

fn missing_target_table_schema(table_name: &str, source_columns: &[ColumnInfo]) -> TableSchema {
    table_schema_from_columns(table_name, source_columns)
}

fn attach_target_column_types(
    mut result: DataCompareResult,
    column_meta: &[QueryColumnMeta],
) -> DataCompareResult {
    result.column_types = result
        .columns
        .iter()
        .filter_map(|column| {
            column_meta
                .iter()
                .find(|meta| meta.name.eq_ignore_ascii_case(column))
                .map(|meta| (column.clone(), meta.db_type.clone()))
        })
        .collect();
    result
}

fn empty_target_response_from_columns(columns: &[ColumnInfo]) -> TableDataResponse {
    TableDataResponse {
        total_count: 0,
        page: 1,
        page_size: DEFAULT_DATA_COMPARE_PAGE_SIZE,
        duration: 0,
        query_result: QueryResult {
            sql: String::new(),
            columns: columns.iter().map(|column| column.name.clone()).collect(),
            column_meta: columns
                .iter()
                .map(|column| QueryColumnMeta::new(&column.name, &column.data_type))
                .collect(),
            rows: Vec::new(),
            binary_cells: Vec::new(),
            elapsed_ms: 0,
        },
    }
}

pub fn resolve_key_columns(
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
                anyhow::bail!("Key column `{key_column}` does not exist on source table");
            };
            if !target_names.contains_key(&key) {
                anyhow::bail!("Key column `{key_column}` does not exist on target table");
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

pub fn resolve_key_columns_for_table(
    requested: &[String],
    source_columns: &[ColumnInfo],
    target_columns: &[ColumnInfo],
    case_sensitive_identifiers: bool,
    pair: &DataCompareTablePair,
) -> anyhow::Result<Vec<String>> {
    resolve_key_columns(
        requested,
        source_columns,
        target_columns,
        case_sensitive_identifiers,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "Key columns for `{}` -> `{}`: {error}",
            pair.source_table,
            pair.target_table
        )
    })
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
                "Duplicate case-insensitive column names in {scope}: `{previous}` and `{name}`"
            );
        }
    }
    Ok(())
}

pub fn quoted_order_by_clause(
    plugin: &dyn DatabasePlugin,
    key_columns: &[String],
) -> Option<String> {
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

fn remap_data_compare_result_to_target_columns(
    mut result: DataCompareResult,
    mappings: &[DataCompareColumnMapping],
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
        .map(|row| super::DataCompareModifiedRow {
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
        })
        .collect();
    result
}

fn remap_row_to_target_columns(row: RowData, mappings: &[DataCompareColumnMapping]) -> RowData {
    mappings
        .iter()
        .filter_map(|mapping| {
            row.get(&mapping.source)
                .cloned()
                .map(|value| (mapping.target.clone(), value))
        })
        .collect()
}

fn target_column_name(source_column: &str, mappings: &[DataCompareColumnMapping]) -> String {
    mappings
        .iter()
        .find(|mapping| mapping.source == source_column)
        .map(|mapping| mapping.target.clone())
        .unwrap_or_else(|| source_column.to_string())
}

pub fn table_lookup_key(value: &str, case_sensitive_identifiers: bool) -> String {
    let value = value.trim();
    let value = match (value.chars().next(), value.chars().last()) {
        (Some('`'), Some('`')) | (Some('"'), Some('"')) | (Some('['), Some(']'))
            if value.len() >= 2 =>
        {
            &value[1..value.len() - 1]
        }
        _ => value,
    };
    identifier_key(value, case_sensitive_identifiers)
}

fn successful_target_table_keys(
    table_results: &[DataCompareResult],
    case_sensitive_identifiers: bool,
) -> HashSet<String> {
    table_results
        .iter()
        .map(|result| table_lookup_key(&result.target_table, case_sensitive_identifiers))
        .collect()
}

fn missing_target_table_keys(
    table_results: &[DataCompareResult],
    case_sensitive_identifiers: bool,
) -> HashSet<String> {
    table_results
        .iter()
        .filter(|result| result.target_table_missing)
        .map(|result| table_lookup_key(&result.target_table, case_sensitive_identifiers))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_info(name: &str, object_type: TableObjectType) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            object_type,
            schema: None,
            comment: None,
            engine: None,

            create_time: None,
            charset: None,
            collation: None,
        }
    }

    #[test]
    fn table_lookup_key_does_not_drop_dots_inside_identifiers() {
        assert_eq!(table_lookup_key("`a.b`", false), "a.b");
        assert_eq!(table_lookup_key("a.b", false), "a.b");
    }

    #[test]
    fn quoted_order_by_clause_quotes_keys_in_order() {
        let plugin = crate::mysql::MySqlPlugin::default();
        let keys = vec!["id".to_string(), "created_at".to_string()];
        assert_eq!(
            quoted_order_by_clause(&plugin, &keys).as_deref(),
            Some("`id`, `created_at`")
        );
    }

    #[test]
    fn matching_table_preserves_object_type_and_respects_case_sensitivity() {
        let tables = vec![
            table_info("users", TableObjectType::Table),
            table_info("AuditLog", TableObjectType::View),
        ];

        assert_eq!(
            matching_table(&tables, "users", true).map(|table| table.object_type),
            Some(TableObjectType::Table)
        );
        assert_eq!(
            matching_table(&tables, "auditlog", false).map(|table| table.object_type),
            Some(TableObjectType::View)
        );
        assert!(matching_table(&tables, "auditlog", true).is_none());
        assert!(matching_table(&tables, "missing", false).is_none());
    }

    #[test]
    fn cross_schema_foreign_keys_are_external_dependencies() {
        let target_tables =
            HashMap::from([(table_lookup_key("users", false), "users".to_string())]);
        let mut dependencies = Vec::new();
        let mut seen = HashSet::new();
        collect_table_dependencies(
            &mut dependencies,
            &mut seen,
            &target_tables,
            "orders",
            vec![ForeignKeyDefinition {
                name: "fk_orders_user".to_string(),
                columns: vec!["user_id".to_string()],
                ref_table: "users".to_string(),
                ref_schema: Some("audit".to_string()),
                ref_columns: vec!["id".to_string()],
                on_delete: String::new(),
                on_update: String::new(),
            }],
            DataCompareDependencyScope {
                source_database: "source_app",
                source_schema: Some("source_public"),
                target_database: "app",
                target_schema: Some("public"),
                case_sensitive_identifiers: false,
            },
        );

        assert_eq!(
            dependencies,
            vec![DataCompareTableDependency {
                table: "orders".to_string(),
                referenced_table: "audit.users".to_string(),
            }]
        );
    }

    #[test]
    fn foreign_keys_using_target_database_namespace_match_selected_tables() {
        let target_tables = HashMap::from([(
            table_lookup_key("QRTZ_TRIGGERS", false),
            "QRTZ_TRIGGERS".to_string(),
        )]);
        let mut dependencies = Vec::new();
        let mut seen = HashSet::new();

        collect_table_dependencies(
            &mut dependencies,
            &mut seen,
            &target_tables,
            "QRTZ_BLOB_TRIGGERS",
            vec![ForeignKeyDefinition {
                name: "FK_BLOB_TRIGGER".to_string(),
                columns: vec!["TRIGGER_NAME".to_string()],
                ref_table: "QRTZ_TRIGGERS".to_string(),
                ref_schema: Some("comi_app_test".to_string()),
                ref_columns: vec!["TRIGGER_NAME".to_string()],
                on_delete: String::new(),
                on_update: String::new(),
            }],
            DataCompareDependencyScope {
                source_database: "source_app",
                source_schema: Some("source_public"),
                target_database: "comi_app_test",
                target_schema: Some("public"),
                case_sensitive_identifiers: false,
            },
        );

        assert_eq!(
            dependencies,
            vec![DataCompareTableDependency {
                table: "QRTZ_BLOB_TRIGGERS".to_string(),
                referenced_table: "QRTZ_TRIGGERS".to_string(),
            }]
        );
    }

    #[test]
    fn foreign_keys_using_source_database_namespace_match_selected_target_tables() {
        let target_tables = HashMap::from([(
            table_lookup_key("QRTZ_TRIGGERS", false),
            "QRTZ_TRIGGERS".to_string(),
        )]);
        let mut dependencies = Vec::new();
        let mut seen = HashSet::new();

        collect_table_dependencies(
            &mut dependencies,
            &mut seen,
            &target_tables,
            "QRTZ_BLOB_TRIGGERS",
            vec![ForeignKeyDefinition {
                name: "FK_BLOB_TRIGGER".to_string(),
                columns: vec!["TRIGGER_NAME".to_string()],
                ref_table: "QRTZ_TRIGGERS".to_string(),
                ref_schema: Some("comi_app_test".to_string()),
                ref_columns: vec!["TRIGGER_NAME".to_string()],
                on_delete: String::new(),
                on_update: String::new(),
            }],
            DataCompareDependencyScope {
                source_database: "comi_app_test",
                source_schema: None,
                target_database: "sync_test",
                target_schema: None,
                case_sensitive_identifiers: false,
            },
        );

        assert_eq!(
            dependencies,
            vec![DataCompareTableDependency {
                table: "QRTZ_BLOB_TRIGGERS".to_string(),
                referenced_table: "QRTZ_TRIGGERS".to_string(),
            }]
        );
    }

    #[test]
    fn successful_target_table_keys_only_contains_successful_results() {
        let results = vec![
            DataCompareResult {
                target_table: "Users".to_string(),
                ..Default::default()
            },
            DataCompareResult {
                target_table: "`Audit.Log`".to_string(),
                ..Default::default()
            },
        ];

        assert_eq!(
            successful_target_table_keys(&results, false),
            HashSet::from(["users".to_string(), "audit.log".to_string()])
        );
        assert_eq!(
            successful_target_table_keys(&results[..1], true),
            HashSet::from(["Users".to_string()])
        );
    }

    #[test]
    fn snapshot_support_matrix_is_conservative() {
        assert_eq!(
            data_compare_snapshot_strategy(&DatabaseType::PostgreSQL),
            DataCompareSnapshotStrategy::Transaction(&[
                "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY"
            ])
        );
        assert_eq!(
            data_compare_snapshot_strategy(&DatabaseType::MySQL),
            DataCompareSnapshotStrategy::Transaction(&[
                "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
                "START TRANSACTION READ ONLY",
            ])
        );
        assert_eq!(
            data_compare_snapshot_strategy(&DatabaseType::SQLite),
            DataCompareSnapshotStrategy::Transaction(&["BEGIN"])
        );

        for database_type in [
            DatabaseType::MSSQL,
            DatabaseType::ClickHouse,
            DatabaseType::DuckDB,
            DatabaseType::Oracle,
            DatabaseType::external("example"),
        ] {
            assert!(matches!(
                data_compare_snapshot_strategy(&database_type),
                DataCompareSnapshotStrategy::BestEffort(_)
            ));
        }
    }

    #[test]
    fn missing_target_columns_are_mapped_to_the_target_database_and_keep_metadata() {
        let source_columns = vec![ColumnInfo {
            name: "amount".to_string(),
            data_type: "INT".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: Some("7".to_string()),
            comment: Some("important amount".to_string()),
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_bin".to_string()),
        }];

        let mapped = map_missing_target_columns(
            &source_columns,
            &DatabaseType::MySQL,
            &DatabaseType::PostgreSQL,
        )
        .unwrap();

        assert_eq!(mapped[0].data_type, "INTEGER");
        assert_eq!(mapped[0].name, source_columns[0].name);
        assert_eq!(mapped[0].is_nullable, source_columns[0].is_nullable);
        assert_eq!(mapped[0].is_primary_key, source_columns[0].is_primary_key);
        assert_eq!(mapped[0].default_value, source_columns[0].default_value);
        assert_eq!(mapped[0].comment, source_columns[0].comment);
        assert_eq!(mapped[0].charset, source_columns[0].charset);
        assert_eq!(mapped[0].collation, source_columns[0].collation);
    }

    #[test]
    fn missing_target_columns_reject_unsupported_cross_database_types() {
        let error = map_missing_target_columns(
            &[ColumnInfo {
                name: "tags".to_string(),
                data_type: "Array(Int32)".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
            }],
            &DatabaseType::ClickHouse,
            &DatabaseType::PostgreSQL,
        )
        .unwrap_err();

        assert!(error.to_string().contains("tags"));
        assert!(error.to_string().contains("Array(Int32)"));
        assert!(error.to_string().contains("无法安全映射"));
    }

    #[test]
    fn missing_target_columns_reject_lossy_cross_database_types() {
        let error = map_missing_target_columns(
            &[ColumnInfo {
                name: "occurred_at".to_string(),
                data_type: "TIMESTAMPTZ(9)".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
            }],
            &DatabaseType::PostgreSQL,
            &DatabaseType::MySQL,
        )
        .unwrap_err();

        assert!(error.to_string().contains("occurred_at"));
        assert!(error.to_string().contains("TIMESTAMPTZ(9)"));
        assert!(error.to_string().contains("无法安全映射"));
        assert!(error.to_string().contains("精度"));
    }
}
