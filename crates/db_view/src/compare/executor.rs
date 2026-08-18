use db::GlobalDbState;
use db::compare::{CompareSchemaSide, CompareTaskEvent, SchemaCompareResult, SyncPlan};
pub use db::compare::{
    DataCompareBatchResult, DataCompareBatchWarning, DataCompareBatchWarningKind,
    DataCompareLimits, DataCompareParams, DataCompareTableDependency, DataCompareTableFailure,
    DataCompareTablePair, SchemaCompareParams,
};
use gpui::AsyncApp;
use rust_i18n::t;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::compare::CompareProgress;

#[cfg(test)]
use db::TableDataResponse;
#[cfg(test)]
use db::compare::{
    DataCompareResult, RowData, append_table_data_page, build_data_compare_result,
    build_missing_target_table_result, quoted_order_by_clause, record_data_compare_pair_result,
    record_dependency_metadata_failure, resolve_key_columns, resolve_key_columns_for_table,
    strip_internal_compare_columns,
};

fn report(progress_tx: &mpsc::UnboundedSender<CompareProgress>, progress: CompareProgress) {
    // 接收端可能因取消而提前关闭,忽略发送错误
    let _ = progress_tx.send(progress);
}

/// 执行数据比较任务。
///
/// 数据库连接、分页、类型转换和依赖加载全部由 `crates/db` 的核心编排负责；
/// 这一层只把结构化事件转换成 UI 进度消息。
pub async fn execute_data_compare(
    params: DataCompareParams,
    db_state: Arc<GlobalDbState>,
    progress_tx: mpsc::UnboundedSender<CompareProgress>,
    cx: &mut AsyncApp,
) -> anyhow::Result<DataCompareBatchResult> {
    db_state
        .prepare_data_compare_from_tables(cx, params, |event| {
            report_data_compare_event(&progress_tx, event);
        })
        .await
}

fn report_data_compare_event(
    progress_tx: &mpsc::UnboundedSender<CompareProgress>,
    event: CompareTaskEvent,
) {
    match event {
        CompareTaskEvent::TableStarted {
            table_index,
            total_tables,
            ..
        } => report(
            progress_tx,
            CompareProgress::steps(
                t!("Compare.comparing_data").to_string(),
                table_index,
                total_tables,
            ),
        ),
        CompareTaskEvent::LoadingMetadata { table } => {
            let phase = match table {
                Some(table) => t!("Compare.loading_metadata_for_table", table = table).to_string(),
                None => t!("Compare.loading_metadata").to_string(),
            };
            report(progress_tx, CompareProgress::phase(phase));
        }
        CompareTaskEvent::LoadingDependencyMetadata { table } => {
            let phase = match table {
                Some(table) => t!(
                    "Compare.loading_dependency_metadata_for_table",
                    table = table
                )
                .to_string(),
                None => t!("Compare.loading_dependency_metadata").to_string(),
            };
            report(progress_tx, CompareProgress::phase(phase));
        }
        CompareTaskEvent::CountingRows { side, .. } => {
            let label = match side {
                db::compare::CompareRowSide::Source => {
                    t!("Compare.loading_source_table_data").to_string()
                }
                db::compare::CompareRowSide::Target => {
                    t!("Compare.loading_target_table_data").to_string()
                }
            };
            report(progress_tx, CompareProgress::phase(label));
        }
        CompareTaskEvent::FetchingRows {
            side,
            fetched_rows,
            total_rows,
            ..
        } => {
            let label = match side {
                db::compare::CompareRowSide::Source => {
                    t!("Compare.loading_source_table_data").to_string()
                }
                db::compare::CompareRowSide::Target => {
                    t!("Compare.loading_target_table_data").to_string()
                }
            };
            match total_rows {
                Some(total) if total > 0 => report(
                    progress_tx,
                    CompareProgress::steps(label, fetched_rows, total),
                ),
                _ => report(progress_tx, CompareProgress::phase(label)),
            }
        }
        CompareTaskEvent::ComparingRows { .. } => report(
            progress_tx,
            CompareProgress::phase(t!("Compare.comparing_data").to_string()),
        ),
        CompareTaskEvent::Error { table, message } => {
            report(
                progress_tx,
                compare_error_progress(table.as_deref(), &message),
            );
        }
        CompareTaskEvent::Started { .. }
        | CompareTaskEvent::TableFinished { .. }
        | CompareTaskEvent::Finished { .. }
        | CompareTaskEvent::LoadingTableList { .. }
        | CompareTaskEvent::LoadingTableSchema { .. }
        | CompareTaskEvent::ComparingSchema
        | CompareTaskEvent::PlanningSql { .. } => {}
    }
}

/// 执行数据比较任务（简化版本）
pub fn generate_data_sync_plan(result: &DataCompareBatchResult) -> SyncPlan {
    db::compare::build_data_sync_batch_plan(result)
}

pub fn generate_data_sync_plan_for_target(
    result: &DataCompareBatchResult,
    db_state: &GlobalDbState,
    target_connection_id: &str,
    target_database: &str,
    target_schema: Option<&str>,
) -> anyhow::Result<SyncPlan> {
    db_state.prepare_data_sync_plan_for_target(
        result,
        target_connection_id,
        target_database,
        target_schema,
    )
}

pub async fn execute_schema_compare(
    params: SchemaCompareParams,
    db_state: Arc<GlobalDbState>,
    progress_tx: mpsc::UnboundedSender<CompareProgress>,
    cx: &mut AsyncApp,
) -> anyhow::Result<SchemaCompareResult> {
    db_state
        .prepare_schema_compare_from_targets(cx, params, |event| {
            report_schema_compare_event(&progress_tx, event);
        })
        .await
}

fn report_schema_compare_event(
    progress_tx: &mpsc::UnboundedSender<CompareProgress>,
    event: CompareTaskEvent,
) {
    match event {
        CompareTaskEvent::LoadingTableList { side } => report(
            progress_tx,
            CompareProgress::phase(
                t!("Compare.loading_table_list", side = schema_side_label(side)).to_string(),
            ),
        ),
        CompareTaskEvent::LoadingTableSchema {
            side,
            table_index,
            total_tables,
            ..
        } => report(
            progress_tx,
            CompareProgress::steps(
                t!(
                    "Compare.reading_table_schema",
                    side = schema_side_label(side)
                )
                .to_string(),
                table_index,
                total_tables,
            ),
        ),
        CompareTaskEvent::ComparingSchema => report(
            progress_tx,
            CompareProgress::phase(t!("Compare.comparing_schema").to_string()),
        ),
        CompareTaskEvent::Error { table, message } => {
            report(
                progress_tx,
                compare_error_progress(table.as_deref(), &message),
            );
        }
        CompareTaskEvent::Started { .. }
        | CompareTaskEvent::TableStarted { .. }
        | CompareTaskEvent::LoadingMetadata { .. }
        | CompareTaskEvent::LoadingDependencyMetadata { .. }
        | CompareTaskEvent::CountingRows { .. }
        | CompareTaskEvent::FetchingRows { .. }
        | CompareTaskEvent::ComparingRows { .. }
        | CompareTaskEvent::PlanningSql { .. }
        | CompareTaskEvent::TableFinished { .. }
        | CompareTaskEvent::Finished { .. } => {}
    }
}

fn compare_error_progress(table: Option<&str>, message: &str) -> CompareProgress {
    let phase = match table {
        Some(table) if !table.is_empty() => t!(
            "Compare.table_compare_failed",
            table = table,
            error = message
        )
        .to_string(),
        _ => t!("Compare.compare_failed", error = message).to_string(),
    };
    CompareProgress::phase(phase)
}

fn schema_side_label(side: CompareSchemaSide) -> String {
    match side {
        CompareSchemaSide::Source => t!("Compare.source").to_string(),
        CompareSchemaSide::Target => t!("Compare.target").to_string(),
    }
}

pub fn generate_schema_sync_plan_for_target(
    result: &SchemaCompareResult,
    db_state: &GlobalDbState,
    source_connection_id: &str,
    target_connection_id: &str,
    target_database: &str,
    target_schema: Option<&str>,
    compare_column_order: bool,
    type_mapping_overrides: db::compare::TypeMappingOverrides,
) -> anyhow::Result<SyncPlan> {
    db_state.prepare_schema_sync_plan_for_target(
        result,
        source_connection_id,
        target_connection_id,
        target_database,
        target_schema,
        compare_column_order,
        type_mapping_overrides,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{CompareSyncExecutionOptions, CompareTargetScope, execute_sync_sql};
    use db::compare::{
        DiffStatus, SchemaCompareTableFailure, SyncStatementKind, rows_from_query_result,
    };
    use db::{BinaryCell, ColumnInfo, ExecOptions, QueryColumnMeta, QueryResult, SqlResult};
    use gpui::TestAppContext;
    use one_core::storage::{DatabaseType, DbConnectionConfig};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn data_compare_error_event_reports_table_and_message() {
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

        report_data_compare_event(
            &progress_tx,
            CompareTaskEvent::Error {
                table: Some("orders".to_string()),
                message: "permission denied".to_string(),
            },
        );

        let progress = progress_rx
            .try_recv()
            .expect("error progress should be sent");
        let label = progress.label();
        assert!(label.contains("orders"));
        assert!(label.contains("permission denied"));
        assert_eq!(progress.percentage(), None);
    }

    #[test]
    fn data_compare_counting_source_rows_reports_source_phase() {
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

        report_data_compare_event(
            &progress_tx,
            CompareTaskEvent::CountingRows {
                table: "users".to_string(),
                side: db::compare::CompareRowSide::Source,
            },
        );

        let progress = progress_rx
            .try_recv()
            .expect("source count progress should be sent");
        assert_eq!(
            progress.label(),
            t!("Compare.loading_source_table_data").as_ref()
        );
        assert_eq!(progress.percentage(), None);
    }

    #[test]
    fn data_compare_counting_target_rows_reports_target_phase() {
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

        report_data_compare_event(
            &progress_tx,
            CompareTaskEvent::CountingRows {
                table: "users".to_string(),
                side: db::compare::CompareRowSide::Target,
            },
        );

        let progress = progress_rx
            .try_recv()
            .expect("target count progress should be sent");
        assert_eq!(
            progress.label(),
            t!("Compare.loading_target_table_data").as_ref()
        );
        assert_eq!(progress.percentage(), None);
    }

    #[test]
    fn data_compare_dependency_metadata_reports_distinct_phase() {
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

        report_data_compare_event(
            &progress_tx,
            CompareTaskEvent::LoadingDependencyMetadata { table: None },
        );

        let progress = progress_rx
            .try_recv()
            .expect("dependency metadata progress should be sent");
        assert_eq!(
            progress.label(),
            t!("Compare.loading_dependency_metadata").as_ref()
        );
        assert_ne!(progress.label(), t!("Compare.loading_metadata").as_ref());
    }

    #[test]
    fn data_compare_table_dependency_metadata_includes_table_name() {
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

        report_data_compare_event(
            &progress_tx,
            CompareTaskEvent::LoadingDependencyMetadata {
                table: Some("orders".to_string()),
            },
        );

        let progress = progress_rx
            .try_recv()
            .expect("table dependency metadata progress should be sent");
        assert!(progress.label().contains("orders"));
    }

    #[test]
    fn schema_compare_error_event_reports_generic_message_without_table() {
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

        report_schema_compare_event(
            &progress_tx,
            CompareTaskEvent::Error {
                table: None,
                message: "table list unavailable".to_string(),
            },
        );

        let progress = progress_rx
            .try_recv()
            .expect("error progress should be sent");
        assert!(progress.label().contains("table list unavailable"));
        assert_eq!(progress.percentage(), None);
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
            binary_cells: vec![],
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
            binary_cells: vec![],
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
            binary_cells: vec![],
            elapsed_ms: 0,
        };

        let rows = rows_from_query_result(&result);

        assert_eq!(
            rows[0].get("price").map(ToString::to_string).as_deref(),
            Some("12345678901234567890.12345")
        );
    }

    #[test]
    fn strip_internal_compare_columns_removes_rowid_and_remaps_binary_cells() {
        let response = TableDataResponse {
            total_count: 1,
            page: 1,
            page_size: 10,
            duration: 0,
            query_result: QueryResult {
                sql: "select rowid as __rowid__, id, payload from users".to_string(),
                columns: vec![
                    "__rowid__".to_string(),
                    "id".to_string(),
                    "payload".to_string(),
                ],
                column_meta: vec![
                    QueryColumnMeta::new("__rowid__", "bigint"),
                    QueryColumnMeta::new("id", "bigint"),
                    QueryColumnMeta::new("payload", "blob"),
                ],
                rows: vec![vec![
                    Some("99".to_string()),
                    Some("1".to_string()),
                    Some("<binary>".to_string()),
                ]],
                binary_cells: vec![
                    BinaryCell {
                        row_index: 0,
                        column_index: 0,
                        bytes: vec![9, 9],
                    },
                    BinaryCell {
                        row_index: 0,
                        column_index: 2,
                        bytes: vec![1, 2, 3],
                    },
                ],
                elapsed_ms: 0,
            },
        };

        let response = strip_internal_compare_columns(response);

        assert_eq!(response.query_result.columns, vec!["id", "payload"]);
        assert_eq!(
            response.query_result.rows,
            vec![vec![Some("1".to_string()), Some("<binary>".to_string())]]
        );
        assert_eq!(
            response.query_result.binary_cells,
            vec![BinaryCell {
                row_index: 0,
                column_index: 1,
                bytes: vec![1, 2, 3],
            }]
        );
    }

    #[test]
    fn append_table_data_page_merges_all_rows_and_binary_offsets() {
        let mut first = table_data_response(
            vec!["id", "payload"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("payload", "blob"),
            ],
            vec![
                vec![Some("1".to_string()), Some("a".to_string())],
                vec![Some("2".to_string()), Some("b".to_string())],
            ],
        );
        first.total_count = 3;
        first.page_size = 2;
        first.query_result.binary_cells = vec![BinaryCell {
            row_index: 1,
            column_index: 1,
            bytes: vec![2],
        }];
        let mut second = table_data_response(
            vec!["id", "payload"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("payload", "blob"),
            ],
            vec![vec![Some("3".to_string()), Some("c".to_string())]],
        );
        second.total_count = 3;
        second.page = 2;
        second.page_size = 2;
        second.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 1,
            bytes: vec![3],
        }];
        let mut accumulated = None;

        append_table_data_page(&mut accumulated, first).unwrap();
        append_table_data_page(&mut accumulated, second).unwrap();

        let accumulated = accumulated.unwrap();
        assert_eq!(accumulated.query_result.rows.len(), 3);
        assert_eq!(
            accumulated.query_result.binary_cells,
            vec![
                BinaryCell {
                    row_index: 1,
                    column_index: 1,
                    bytes: vec![2],
                },
                BinaryCell {
                    row_index: 2,
                    column_index: 1,
                    bytes: vec![3],
                },
            ]
        );
    }

    #[test]
    fn append_table_data_page_rejects_count_changes() {
        let mut first = table_data_response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "int")],
            vec![vec![Some("1".to_string())]],
        );
        first.total_count = 2;
        let mut second = table_data_response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "int")],
            vec![vec![Some("2".to_string())]],
        );
        second.total_count = 3;
        let mut accumulated = None;

        append_table_data_page(&mut accumulated, first).unwrap();
        let error = append_table_data_page(&mut accumulated, second).unwrap_err();

        assert!(error.to_string().contains("row count changed"));
    }

    #[test]
    fn build_data_compare_result_never_treats_internal_rowid_as_business_data() {
        let source = table_data_response(
            vec!["__rowid__", "id", "name"],
            vec![
                QueryColumnMeta::new("__rowid__", "bigint"),
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![
                Some("10".to_string()),
                Some("1".to_string()),
                Some("Ada".to_string()),
            ]],
        );
        let target = table_data_response(
            vec!["__rowid__", "id", "name"],
            vec![
                QueryColumnMeta::new("__rowid__", "bigint"),
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![
                Some("999".to_string()),
                Some("1".to_string()),
                Some("Ada".to_string()),
            ]],
        );

        let source = strip_internal_compare_columns(source);
        let target = strip_internal_compare_columns(target);
        let result = build_data_compare_result(
            DataCompareTablePair {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
            },
            vec!["id".to_string()],
            source,
            target,
            false,
        )
        .unwrap();

        assert_eq!(result.columns, vec!["id", "name"]);
        assert!(!result.column_types.contains_key("__rowid__"));
        assert!(result.modified.is_empty());
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
    fn resolve_key_columns_for_table_rejects_invalid_requested_key_in_multi_table_compare() {
        let source = vec![column_info("id", true), column_info("tenant_id", false)];
        let target = vec![column_info("id", true)];
        let requested = vec!["tenant_id".to_string()];

        let result = resolve_key_columns_for_table(
            &requested,
            &source,
            &target,
            false,
            &DataCompareTablePair {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
            },
        )
        .unwrap_err();

        assert!(result.to_string().contains("tenant_id"));
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
        assert_eq!(
            table_result.column_types.get("name"),
            Some(&"text".to_string())
        );
        let plan = generate_data_sync_plan(&DataCompareBatchResult {
            table_results: vec![table_result],
            table_dependencies: vec![],
            ..Default::default()
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
    fn record_data_compare_pair_result_isolates_table_failures() {
        let mut results = Vec::new();
        let mut failures = Vec::new();

        record_data_compare_pair_result(
            &mut results,
            &mut failures,
            "users".to_string(),
            Ok(DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![row_data(vec![("id", json!(1))])],
                ..Default::default()
            }),
        );
        record_data_compare_pair_result(
            &mut results,
            &mut failures,
            "orders".to_string(),
            Err(anyhow::anyhow!("source query failed")),
        );

        assert_eq!(results.len(), 1);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].table, "orders");
        assert!(failures[0].error.contains("source query failed"));
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
                ..Default::default()
            }],
            table_dependencies: vec![],
            ..Default::default()
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
    fn generate_data_sync_plan_blocks_truncated_missing_target_results() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![row_data(vec![("id", json!(1))])],
                source_truncated: true,
                target_table_missing: true,
                missing_target_schema: Some(db::compare::TableSchema {
                    name: "users".to_string(),
                    columns: vec![db::compare::ColumnSchema {
                        name: "id".to_string(),
                        data_type: "int".to_string(),
                        nullable: false,
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let plan = generate_data_sync_plan(&result);

        assert_eq!(plan.summary.total_count, 0);
        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert!(!plan.sql_text.contains("CREATE TABLE"));
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("truncated"))
        );
    }

    #[test]
    fn targeted_data_sync_plan_blocks_truncated_results_without_live_connection() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![row_data(vec![("id", json!(1))])],
                target_truncated: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        let plan = generate_data_sync_plan_for_target(
            &result,
            &GlobalDbState::new(),
            "missing-connection",
            "app",
            None,
        )
        .expect("blocked plans must not require a live target connection");

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
    fn targeted_schema_sync_plan_blocks_incomplete_results_without_live_connection() {
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_diffs: vec![],
            table_failures: vec![SchemaCompareTableFailure {
                side: CompareSchemaSide::Target,
                table: "orders".to_string(),
                error: "permission denied".to_string(),
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 0,
        };

        let plan = generate_schema_sync_plan_for_target(
            &result,
            &GlobalDbState::new(),
            "missing-connection",
            "missing-connection",
            "app",
            None,
            false,
        )
        .expect("blocked plans must not require a live target connection");

        assert_eq!(plan.summary.total_count, 0);
        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("orders") && warning.contains("permission denied"))
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
                    ..Default::default()
                },
                DataCompareResult {
                    source_table: "orders".to_string(),
                    target_table: "orders".to_string(),
                    key_columns: vec!["id".to_string()],
                    columns: vec!["id".to_string(), "total".to_string()],
                    added: vec![row_data(vec![("id", json!(7)), ("total", json!(19.5))])],
                    removed: vec![],
                    modified: vec![],
                    ..Default::default()
                },
            ],
            table_dependencies: vec![],
            ..Default::default()
        };

        let plan = generate_data_sync_plan(&result);

        assert_eq!(plan.target_table, "2 tables");
        assert_eq!(plan.summary.insert_count, 2);
        assert_eq!(plan.summary.total_count, 2);
        assert!(plan.sql_text.contains("INSERT INTO users"));
        assert!(plan.sql_text.contains("INSERT INTO orders"));
    }

    #[test]
    fn generate_data_sync_plan_reports_failed_tables_and_keeps_successful_ones() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![row_data(vec![("id", json!(1))])],
                removed: vec![],
                modified: vec![],
                ..Default::default()
            }],
            table_dependencies: vec![],
            table_failures: vec![DataCompareTableFailure {
                table: "orders".to_string(),
                error: "source query failed".to_string(),
            }],
            ..Default::default()
        };

        assert!(result.has_failed_tables());
        let plan = generate_data_sync_plan(&result);

        assert_eq!(plan.summary.insert_count, 1);
        assert!(plan.sql_text.contains("INSERT INTO users"));
        assert!(!plan.sql_text.contains("orders"));
        assert!(
            plan.warnings.iter().any(
                |warning| warning.contains("orders") && warning.contains("source query failed")
            )
        );
    }

    #[test]
    fn record_dependency_metadata_failure_preserves_context() {
        let mut warnings = Vec::new();

        record_dependency_metadata_failure(
            &mut warnings,
            Some("orders".to_string()),
            DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable,
            anyhow::anyhow!("foreign key query failed"),
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].kind,
            DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable
        );
        assert_eq!(warnings[0].table.as_deref(), Some("orders"));
        assert!(warnings[0].error.contains("foreign key query failed"));
    }

    #[test]
    fn generate_data_sync_plan_blocks_incomplete_dependency_metadata() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![row_data(vec![("id", json!(1))])],
                ..Default::default()
            }],
            batch_warnings: vec![DataCompareBatchWarning {
                table: Some("users".to_string()),
                kind: DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable,
                error: "fk metadata unavailable".to_string(),
            }],
            ..Default::default()
        };

        assert!(result.has_incomplete_dependency_metadata());
        assert!(result.is_sync_sql_blocked());
        assert_eq!(result.table_results.len(), 1);

        let plan = generate_data_sync_plan(&result);

        assert_eq!(plan.summary.total_count, 0);
        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert!(plan.warnings.iter().any(|warning| {
            warning.contains("Dependency metadata") && warning.contains("statement ordering")
        }));
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("fk metadata unavailable"))
        );
    }

    #[test]
    fn generate_data_sync_plan_blocks_when_target_table_metadata_is_unavailable() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![row_data(vec![("id", json!(1))])],
                ..Default::default()
            }],
            batch_warnings: vec![DataCompareBatchWarning {
                table: None,
                kind: DataCompareBatchWarningKind::TargetTableMetadataUnavailable,
                error: "table metadata unavailable".to_string(),
            }],
            ..Default::default()
        };

        let plan = generate_data_sync_plan(&result);
        let targeted_plan = generate_data_sync_plan_for_target(
            &result,
            &GlobalDbState::new(),
            "missing-connection",
            "app",
            None,
        )
        .expect("blocked plans must not require a live target connection");

        for plan in [plan, targeted_plan] {
            assert_eq!(plan.summary.total_count, 0);
            assert!(plan.statements.is_empty());
            assert!(plan.sql_text.is_empty());
            assert!(
                plan.warnings
                    .iter()
                    .any(|warning| warning.contains("table metadata unavailable"))
            );
        }
    }

    #[test]
    fn generate_data_sync_plan_puts_missing_target_create_table_before_inserts() {
        use db::compare::{ColumnSchema, TableSchema};

        let result = DataCompareBatchResult {
            table_results: vec![
                DataCompareResult {
                    source_table: "users".to_string(),
                    target_table: "users".to_string(),
                    key_columns: vec!["id".to_string()],
                    columns: vec!["id".to_string()],
                    added: vec![row_data(vec![("id", json!(1))])],
                    removed: vec![],
                    modified: vec![],
                    target_table_missing: true,
                    missing_target_schema: Some(TableSchema {
                        name: "users".to_string(),
                        columns: vec![ColumnSchema {
                            name: "id".to_string(),
                            data_type: "int".to_string(),
                            nullable: false,
                            ..Default::default()
                        }],
                        indexes: vec![],
                        foreign_keys: vec![],
                        comment: None,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                DataCompareResult {
                    source_table: "orders".to_string(),
                    target_table: "orders".to_string(),
                    key_columns: vec!["id".to_string()],
                    columns: vec!["id".to_string()],
                    added: vec![row_data(vec![("id", json!(7))])],
                    removed: vec![],
                    modified: vec![],
                    ..Default::default()
                },
            ],
            table_dependencies: vec![],
            ..Default::default()
        };

        let plan = generate_data_sync_plan(&result);

        assert_eq!(plan.summary.ddl_count, 1);
        assert_eq!(plan.summary.insert_count, 2);
        assert_eq!(plan.summary.total_count, 3);
        assert_sql_order(&plan.sql_text, "CREATE TABLE users", "INSERT INTO users");
        assert_sql_order(&plan.sql_text, "CREATE TABLE users", "INSERT INTO orders");
    }

    #[test]
    fn generate_data_sync_plan_orders_multi_table_rows_by_foreign_key_dependencies() {
        let result = DataCompareBatchResult {
            table_results: vec![
                DataCompareResult {
                    source_table: "users".to_string(),
                    target_table: "users".to_string(),
                    key_columns: vec!["id".to_string()],
                    columns: vec!["id".to_string(), "department_id".to_string()],
                    added: vec![row_data(vec![
                        ("id", json!(10)),
                        ("department_id", json!(3)),
                    ])],
                    removed: vec![row_data(vec![
                        ("id", json!(11)),
                        ("department_id", json!(4)),
                    ])],
                    modified: vec![],
                    ..Default::default()
                },
                DataCompareResult {
                    source_table: "departments".to_string(),
                    target_table: "departments".to_string(),
                    key_columns: vec!["id".to_string()],
                    columns: vec!["id".to_string(), "name".to_string()],
                    added: vec![row_data(vec![("id", json!(3)), ("name", json!("AI"))])],
                    removed: vec![row_data(vec![("id", json!(4)), ("name", json!("Old"))])],
                    modified: vec![],
                    ..Default::default()
                },
            ],
            table_dependencies: vec![DataCompareTableDependency {
                table: "users".to_string(),
                referenced_table: "departments".to_string(),
            }],
            ..Default::default()
        };

        let plan = generate_data_sync_plan(&result);

        assert_sql_order(
            &plan.sql_text,
            "INSERT INTO departments",
            "INSERT INTO users",
        );
        assert_sql_order(&plan.sql_text, "INSERT INTO users", "DELETE FROM users");
        assert_sql_order(
            &plan.sql_text,
            "DELETE FROM users",
            "DELETE FROM departments",
        );
    }

    #[test]
    fn build_missing_target_table_result_marks_all_source_rows_as_added() {
        let source = table_data_response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![
                vec![Some("1".to_string()), Some("Ada".to_string())],
                vec![Some("2".to_string()), Some("Grace".to_string())],
            ],
        );
        let source_columns = vec![column_info("id", true), column_info("name", false)];

        let result = build_missing_target_table_result(
            DataCompareTablePair {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
            },
            vec!["id".to_string()],
            &source_columns,
            source,
            false,
        )
        .unwrap();

        assert_eq!(2, result.added.len());
        assert!(result.removed.is_empty());
        assert!(result.modified.is_empty());
        assert!(result.target_table_missing);
        assert_eq!(result.columns, vec!["id".to_string(), "name".to_string()]);

        let schema = result
            .missing_target_schema
            .as_ref()
            .expect("missing-target 结果必须携带源表结构");
        assert_eq!(schema.name, "users");
        assert_eq!(schema.columns.len(), 2);
        assert!(schema.indexes.iter().any(|index| index.name == "PRIMARY"));
        assert_eq!(result.column_types.get("name"), Some(&"int".to_string()));
    }

    #[test]
    fn generate_data_sync_plan_warns_and_unselects_inserts_with_unselected_parent_table() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "apps".to_string(),
                target_table: "apps".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string(), "user_id".to_string()],
                added: vec![row_data(vec![("id", json!(1)), ("user_id", json!(42))])],
                removed: vec![],
                modified: vec![],
                ..Default::default()
            }],
            table_dependencies: vec![DataCompareTableDependency {
                table: "apps".to_string(),
                referenced_table: "users".to_string(),
            }],
            ..Default::default()
        };

        let plan = generate_data_sync_plan(&result);

        assert_eq!(1, plan.statements.len());
        assert!(!plan.statements[0].selected_by_default);
        assert!(!plan.statements[0].warnings.is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("users"))
        );
    }

    #[gpui::test]
    #[ignore = "requires ONETCLI_TEST_MYSQL_PASSWORD and a local MySQL server"]
    fn mysql_compare_and_sync_uses_real_connection(cx: &mut TestAppContext) {
        let Some(config) = mysql_test_config_from_env() else {
            eprintln!("ONETCLI_TEST_MYSQL_PASSWORD not set; skipping");
            return;
        };
        let mut state = GlobalDbState::new();
        let connection_id = config.id.clone();
        state.register_connection(config);
        let db_state = Arc::new(state);
        cx.update(one_core::gpui_tokio::init);
        cx.executor().allow_parking();

        let executor = cx.foreground_executor().clone();
        let mut async_cx = cx.to_async();
        executor
            .block_on(run_mysql_compare_fixture(
                db_state,
                connection_id,
                &mut async_cx,
            ))
            .unwrap();
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
                binary_cells: vec![],
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

    fn assert_sql_order(sql: &str, first: &str, second: &str) {
        let first_index = sql.find(first).expect("first SQL fragment should exist");
        let second_index = sql.find(second).expect("second SQL fragment should exist");
        assert!(
            first_index < second_index,
            "expected `{first}` before `{second}` in:\n{sql}"
        );
    }

    fn mysql_test_config_from_env() -> Option<DbConnectionConfig> {
        let password = std::env::var("ONETCLI_TEST_MYSQL_PASSWORD").ok()?;
        Some(DbConnectionConfig {
            id: "onetcli-compare-real-mysql".to_string(),
            database_type: DatabaseType::MySQL,
            name: "onetcli compare real mysql".to_string(),
            host: std::env::var("ONETCLI_TEST_MYSQL_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: std::env::var("ONETCLI_TEST_MYSQL_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(3306),
            username: std::env::var("ONETCLI_TEST_MYSQL_USER")
                .unwrap_or_else(|_| "root".to_string()),
            password,
            credential_reference: None,
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: HashMap::new(),
        })
    }

    async fn setup_mysql_fixture(
        db_state: &GlobalDbState,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        exec_mysql_script(db_state, connection_id, None, MYSQL_COMPARE_FIXTURE_SQL, cx)
            .await
            .map(|_| ())
    }

    async fn cleanup_mysql_fixture(
        db_state: &GlobalDbState,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) {
        let _ =
            exec_mysql_script(db_state, connection_id, None, MYSQL_COMPARE_CLEANUP_SQL, cx).await;
    }

    async fn execute_real_schema_compare(
        db_state: &Arc<GlobalDbState>,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<SchemaCompareResult> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        execute_schema_compare(
            SchemaCompareParams {
                source_connection_id: connection_id.to_string(),
                source_database: "onetcli_compare_src".to_string(),
                source_schema: None,
                source_tables: vec!["users".to_string()],
                target_connection_id: connection_id.to_string(),
                target_database: "onetcli_compare_dst".to_string(),
                target_schema: None,
                target_tables: vec!["users".to_string()],
                case_sensitive_identifiers: false,
                compare_views: false,
                compare_routines: false,
                compare_triggers: false,
                compare_indexes: true,
                compare_foreign_keys: true,
                ignore_comments: false,
                ignore_auto_increment: false,
                ignore_charset_collation: false,
                ignore_table_options: false,
                compare_column_order: false,
                type_mapping_overrides: db::compare::TypeMappingOverrides::default(),
            },
            db_state.clone(),
            tx,
            cx,
        )
        .await
    }

    async fn run_mysql_compare_fixture(
        db_state: Arc<GlobalDbState>,
        connection_id: String,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        cleanup_mysql_fixture(&db_state, &connection_id, cx).await;
        setup_mysql_fixture(&db_state, &connection_id, cx).await?;
        run_mysql_schema_sync_assertions(&db_state, &connection_id, cx).await?;
        run_mysql_data_sync_assertions(&db_state, &connection_id, cx).await?;
        assert_streaming_error_modes(&db_state, &connection_id, cx).await?;
        cleanup_mysql_fixture(&db_state, &connection_id, cx).await;
        Ok(())
    }

    async fn run_mysql_schema_sync_assertions(
        db_state: &Arc<GlobalDbState>,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        let schema = execute_real_schema_compare(db_state, connection_id, cx).await?;
        assert_schema_fixture_diff(&schema);
        let plan = generate_schema_sync_plan_for_target(
            &schema,
            db_state,
            connection_id,
            connection_id,
            "onetcli_compare_dst",
            None,
            false,
            db::compare::TypeMappingOverrides::default(),
        )?;
        assert_no_display_charset_labels(&plan.sql_text);
        let schema_results = run_sync_sql_and_collect(
            db_state,
            connection_id,
            "onetcli_compare_dst",
            selected_default_sql(&plan),
            CompareSyncExecutionOptions::schema_ddl(false),
            cx,
        )
        .await?;
        assert_no_streaming_errors(&schema_results);
        let synced_schema = execute_real_schema_compare(db_state, connection_id, cx).await?;
        assert_schema_fixture_synced(&synced_schema);
        Ok(())
    }

    async fn run_mysql_data_sync_assertions(
        db_state: &Arc<GlobalDbState>,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        let data = execute_real_data_compare(db_state, connection_id, cx).await?;
        assert_data_fixture_diff(&data);
        let plan = generate_data_sync_plan_for_target(
            &data,
            db_state,
            connection_id,
            "onetcli_compare_dst",
            None,
        )?;
        assert_data_delete_is_destructive_and_not_selected(&plan);
        assert_no_display_charset_labels(&plan.sql_text);
        assert_sql_order(
            &plan.sql_text,
            "INSERT INTO `onetcli_compare_dst`.`departments`",
            "INSERT INTO `onetcli_compare_dst`.`users`",
        );
        assert_sql_order(
            &plan.sql_text,
            "DELETE FROM `onetcli_compare_dst`.`users`",
            "DELETE FROM `onetcli_compare_dst`.`departments`",
        );
        let data_results = run_sync_sql_and_collect(
            db_state,
            connection_id,
            "onetcli_compare_dst",
            plan.sql_text.clone(),
            CompareSyncExecutionOptions::default(),
            cx,
        )
        .await?;
        assert_no_streaming_errors(&data_results);
        assert_target_users_match_source(db_state, connection_id, cx).await?;
        let synced_data = execute_real_data_compare(db_state, connection_id, cx).await?;
        assert_data_fixture_synced(&synced_data);
        Ok(())
    }

    async fn execute_real_data_compare(
        db_state: &Arc<GlobalDbState>,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<DataCompareBatchResult> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        execute_data_compare(
            DataCompareParams {
                source_connection_id: connection_id.to_string(),
                source_database: "onetcli_compare_src".to_string(),
                source_schema: None,
                target_connection_id: connection_id.to_string(),
                target_database: "onetcli_compare_dst".to_string(),
                target_schema: None,
                table_pairs: vec![
                    DataCompareTablePair {
                        source_table: "users".to_string(),
                        target_table: "users".to_string(),
                    },
                    DataCompareTablePair {
                        source_table: "departments".to_string(),
                        target_table: "departments".to_string(),
                    },
                ],
                key_columns: vec![],
                case_sensitive_identifiers: false,
                limits: DataCompareLimits::default(),
            },
            db_state.clone(),
            tx,
            cx,
        )
        .await
    }

    async fn exec_mysql_script(
        db_state: &GlobalDbState,
        connection_id: &str,
        database: Option<&str>,
        sql: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<Vec<SqlResult>> {
        db_state
            .execute_script(
                cx,
                connection_id.to_string(),
                sql.to_string(),
                database.map(ToString::to_string),
                None,
                Some(ExecOptions {
                    stop_on_error: true,
                    transactional: false,
                    max_rows: None,
                    streaming: false,
                }),
            )
            .await
    }

    async fn run_sync_sql_and_collect(
        db_state: &Arc<GlobalDbState>,
        connection_id: &str,
        database: &str,
        sql: String,
        options: CompareSyncExecutionOptions,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<Vec<SqlResult>> {
        let target = CompareTargetScope {
            connection_id: connection_id.to_string(),
            database: database.to_string(),
            schema: None,
        };
        let mut rx = execute_sync_sql(target, sql, db_state.clone(), options, cx)?;
        let mut results = Vec::new();
        while let Some(progress) = rx.recv().await {
            results.push(progress.result);
        }
        Ok(results)
    }

    fn selected_default_sql(plan: &SyncPlan) -> String {
        plan.statements
            .iter()
            .filter(|statement| statement.selected_by_default)
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_schema_fixture_diff(result: &SchemaCompareResult) {
        assert_eq!(1, result.table_diffs.len());
        let users = result
            .table_diffs
            .iter()
            .find(|diff| diff.name == "users")
            .expect("users table diff should exist");
        assert_eq!(DiffStatus::Modified, users.status);
        assert!(users.column_diffs.iter().any(|diff| diff.name == "email"));
        assert!(
            users
                .index_diffs
                .iter()
                .any(|diff| diff.name == "idx_users_email")
        );
        assert!(
            users
                .foreign_key_diffs
                .iter()
                .any(|diff| diff.name == "fk_users_department")
        );
    }

    fn assert_schema_fixture_synced(result: &SchemaCompareResult) {
        assert!(
            result.table_diffs.is_empty(),
            "schema compare should converge after sync: {:?}",
            result.table_diffs
        );
    }

    fn assert_data_fixture_diff(result: &DataCompareBatchResult) {
        let users = table_result(result, "users");
        assert_eq!(1, users.added.len());
        assert_eq!(1, users.removed.len());
        assert!(
            users.modified.iter().any(|row| {
                row.changes.contains_key("email") && row.changes.contains_key("name")
            })
        );
        let departments = table_result(result, "departments");
        assert_eq!(1, departments.added.len());
        assert_eq!(1, departments.removed.len());
        assert!(departments.modified.is_empty());
        assert!(
            result.table_dependencies.iter().any(|dependency| {
                dependency.table == "users" && dependency.referenced_table == "departments"
            }),
            "expected users -> departments dependency"
        );
    }

    fn assert_data_fixture_synced(result: &DataCompareBatchResult) {
        for table in &result.table_results {
            assert!(table.added.is_empty(), "unexpected added rows: {table:?}");
            assert!(
                table.removed.is_empty(),
                "unexpected removed rows: {table:?}"
            );
            assert!(
                table.modified.is_empty(),
                "unexpected modified rows: {table:?}"
            );
        }
    }

    fn table_result<'a>(result: &'a DataCompareBatchResult, table: &str) -> &'a DataCompareResult {
        result
            .table_results
            .iter()
            .find(|table_result| table_result.target_table == table)
            .unwrap_or_else(|| panic!("{table} data diff should exist"))
    }

    fn assert_data_delete_is_destructive_and_not_selected(plan: &SyncPlan) {
        assert!(plan.statements.iter().any(|statement| {
            matches!(statement.kind, SyncStatementKind::Delete)
                && statement.destructive
                && !statement.selected_by_default
        }));
    }

    fn assert_no_display_charset_labels(sql: &str) {
        assert!(!sql.contains("Default UTF-8"));
        assert!(!sql.contains("UTF-8 Unicode"));
        assert!(!sql.contains("Default InnoDB"));
    }

    fn assert_no_streaming_errors(results: &[SqlResult]) {
        let errors = results
            .iter()
            .filter_map(|result| match result {
                SqlResult::Error(error) => Some(error.message.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "streaming SQL errors: {errors:?}");
    }

    async fn assert_target_users_match_source(
        db_state: &GlobalDbState,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        let results = exec_mysql_script(
            db_state,
            connection_id,
            Some("onetcli_compare_dst"),
            TARGET_MATCH_QUERY,
            cx,
        )
        .await?;
        assert_eq!("0", first_query_value(&results));
        Ok(())
    }

    async fn assert_streaming_error_modes(
        db_state: &Arc<GlobalDbState>,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<()> {
        exec_mysql_script(
            db_state,
            connection_id,
            Some("onetcli_compare_dst"),
            ERROR_PROBE_DDL,
            cx,
        )
        .await?;
        let continued = run_sync_sql_and_collect(
            db_state,
            connection_id,
            "onetcli_compare_dst",
            ERROR_PROBE_INSERTS.to_string(),
            CompareSyncExecutionOptions {
                use_transaction: true,
                continue_on_error: true,
            },
            cx,
        )
        .await?;
        assert_eq!(
            2,
            continued.iter().filter(|result| !result.is_error()).count()
        );
        assert_eq!(
            1,
            continued.iter().filter(|result| result.is_error()).count()
        );
        assert_eq!("2", probe_row_count(db_state, connection_id, cx).await?);

        exec_mysql_script(
            db_state,
            connection_id,
            Some("onetcli_compare_dst"),
            ERROR_PROBE_DDL,
            cx,
        )
        .await?;
        let stopped = run_sync_sql_and_collect(
            db_state,
            connection_id,
            "onetcli_compare_dst",
            ERROR_PROBE_INSERTS.to_string(),
            CompareSyncExecutionOptions {
                use_transaction: false,
                continue_on_error: false,
            },
            cx,
        )
        .await?;
        assert_eq!(
            1,
            stopped.iter().filter(|result| !result.is_error()).count()
        );
        assert_eq!(1, stopped.iter().filter(|result| result.is_error()).count());
        assert_eq!("1", probe_row_count(db_state, connection_id, cx).await?);
        Ok(())
    }

    async fn probe_row_count(
        db_state: &GlobalDbState,
        connection_id: &str,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<String> {
        let results = exec_mysql_script(
            db_state,
            connection_id,
            Some("onetcli_compare_dst"),
            "SELECT COUNT(*) FROM error_probe;",
            cx,
        )
        .await?;
        Ok(first_query_value(&results))
    }

    fn first_query_value(results: &[SqlResult]) -> String {
        let Some(SqlResult::Query(query)) = results.first() else {
            panic!("expected a query result");
        };
        query.rows[0][0]
            .clone()
            .expect("query value should not be null")
    }

    const MYSQL_COMPARE_CLEANUP_SQL: &str = r#"
DROP DATABASE IF EXISTS onetcli_compare_src;
DROP DATABASE IF EXISTS onetcli_compare_dst;
"#;

    const MYSQL_COMPARE_FIXTURE_SQL: &str = r#"
DROP DATABASE IF EXISTS onetcli_compare_src;
DROP DATABASE IF EXISTS onetcli_compare_dst;
CREATE DATABASE onetcli_compare_src CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE onetcli_compare_dst CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

CREATE TABLE onetcli_compare_src.departments (
  id INT PRIMARY KEY,
  name VARCHAR(80) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE onetcli_compare_dst.departments (
  id INT PRIMARY KEY,
  name VARCHAR(80) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE onetcli_compare_src.users (
  id INT PRIMARY KEY,
  name VARCHAR(100) NOT NULL,
  email VARCHAR(160) NULL,
  department_id INT NOT NULL,
  status VARCHAR(20) NOT NULL DEFAULT 'active',
  updated_at TIMESTAMP NULL DEFAULT CURRENT_TIMESTAMP,
  INDEX idx_users_email (email),
  CONSTRAINT fk_users_department FOREIGN KEY (department_id) REFERENCES departments(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE onetcli_compare_dst.users (
  id INT PRIMARY KEY,
  name VARCHAR(100) NOT NULL,
  department_id INT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO onetcli_compare_src.departments
VALUES (1, 'Engineering'), (2, 'Sales'), (3, 'Research');
INSERT INTO onetcli_compare_dst.departments
VALUES (1, 'Engineering'), (2, 'Sales'), (4, 'Legacy');
INSERT INTO onetcli_compare_src.users
  (id, name, email, department_id, status)
VALUES
  (1, 'Ada Lovelace', 'ada@example.com', 1, 'active'),
  (2, 'Grace Hopper', 'grace@example.com', 1, 'active'),
  (3, 'Katherine Johnson', 'katherine@example.com', 3, 'inactive');
INSERT INTO onetcli_compare_dst.users (id, name, department_id)
VALUES
  (1, 'Ada', 1),
  (2, 'Grace Hopper', 1),
  (4, 'Legacy User', 4);
"#;

    const TARGET_MATCH_QUERY: &str = r#"
SELECT (
  SELECT COUNT(*)
  FROM onetcli_compare_src.users s
  LEFT JOIN onetcli_compare_dst.users d
    ON s.id = d.id
   AND s.name <=> d.name
   AND s.email <=> d.email
   AND s.department_id <=> d.department_id
   AND s.status <=> d.status
  WHERE d.id IS NULL
) + (
  SELECT COUNT(*)
  FROM onetcli_compare_dst.users d
  LEFT JOIN onetcli_compare_src.users s
    ON s.id = d.id
   AND s.name <=> d.name
   AND s.email <=> d.email
   AND s.department_id <=> d.department_id
   AND s.status <=> d.status
  WHERE s.id IS NULL
) + (
  SELECT COUNT(*)
  FROM onetcli_compare_src.departments s
  LEFT JOIN onetcli_compare_dst.departments d
    ON s.id = d.id
   AND s.name <=> d.name
  WHERE d.id IS NULL
) + (
  SELECT COUNT(*)
  FROM onetcli_compare_dst.departments d
  LEFT JOIN onetcli_compare_src.departments s
    ON s.id = d.id
   AND s.name <=> d.name
  WHERE s.id IS NULL
) AS mismatch_count;
"#;

    const ERROR_PROBE_DDL: &str = r#"
DROP TABLE IF EXISTS error_probe;
CREATE TABLE error_probe (id INT PRIMARY KEY);
"#;

    const ERROR_PROBE_INSERTS: &str = r#"
INSERT INTO error_probe VALUES (1);
INSERT INTO error_probe VALUES (1);
INSERT INTO error_probe VALUES (2);
"#;
}
