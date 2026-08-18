use super::DataCompareResult;

pub const DEFAULT_DATA_COMPARE_MAX_ROWS_PER_TABLE: usize = 1_000_000;
pub const DEFAULT_DATA_COMPARE_MAX_PAGES_PER_TABLE: usize = 100;

/// Safety limits for one side of one table comparison.
///
/// Data comparison still pages until the exact COUNT is reached whenever the
/// table fits within these limits. Reaching a configured limit returns a
/// partial result marked as truncated; the batch-level sync-plan builder then
/// disables SQL generation for the entire batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataCompareLimits {
    pub max_rows_per_table: Option<usize>,
    pub max_pages_per_table: Option<usize>,
}

impl DataCompareLimits {
    pub const fn unlimited() -> Self {
        Self {
            max_rows_per_table: None,
            max_pages_per_table: None,
        }
    }
}

impl Default for DataCompareLimits {
    fn default() -> Self {
        Self {
            max_rows_per_table: Some(DEFAULT_DATA_COMPARE_MAX_ROWS_PER_TABLE),
            max_pages_per_table: Some(DEFAULT_DATA_COMPARE_MAX_PAGES_PER_TABLE),
        }
    }
}

/// Parameters shared by the data-compare orchestrator and its UI clients.
///
/// This intentionally contains connection/table identifiers and comparison
/// policy only. Progress reporting and localization belong to the caller.
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
    pub limits: DataCompareLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCompareTablePair {
    pub source_table: String,
    pub target_table: String,
}

#[derive(Debug, Clone, Default)]
pub struct DataCompareBatchResult {
    pub table_results: Vec<DataCompareResult>,
    pub table_dependencies: Vec<DataCompareTableDependency>,
    pub table_failures: Vec<DataCompareTableFailure>,
    pub batch_warnings: Vec<DataCompareBatchWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCompareTableDependency {
    pub table: String,
    pub referenced_table: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCompareTableFailure {
    pub table: String,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataCompareBatchWarningKind {
    TargetTableMetadataUnavailable,
    ForeignKeyMetadataUnavailable,
    ConsistentSnapshotUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCompareBatchWarning {
    pub table: Option<String>,
    pub kind: DataCompareBatchWarningKind,
    pub error: String,
}

impl DataCompareBatchResult {
    pub fn has_truncated_tables(&self) -> bool {
        self.table_results
            .iter()
            .any(|table| table.source_truncated || table.target_truncated)
    }

    pub fn has_missing_target_tables(&self) -> bool {
        self.table_results
            .iter()
            .any(|table| table.target_table_missing)
    }

    pub fn has_failed_tables(&self) -> bool {
        !self.table_failures.is_empty()
    }

    pub fn has_incomplete_dependency_metadata(&self) -> bool {
        self.batch_warnings.iter().any(|warning| {
            matches!(
                warning.kind,
                DataCompareBatchWarningKind::TargetTableMetadataUnavailable
                    | DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable
            )
        })
    }

    pub fn has_inconsistent_snapshot_risk(&self) -> bool {
        self.batch_warnings.iter().any(|warning| {
            warning.kind == DataCompareBatchWarningKind::ConsistentSnapshotUnavailable
        })
    }

    /// A failed table is isolated, so it does not block SQL for successful
    /// tables. Incomplete dependency metadata is different: without it the
    /// generated statement order cannot be trusted.
    pub fn is_sync_sql_blocked(&self) -> bool {
        self.has_truncated_tables()
            || self.has_incomplete_dependency_metadata()
            || self.has_inconsistent_snapshot_risk()
    }
}

/// Parameters shared by the schema-compare orchestrator and its UI clients.
#[derive(Debug, Clone)]
pub struct SchemaCompareParams {
    pub source_connection_id: String,
    pub source_database: String,
    pub source_schema: Option<String>,
    pub source_tables: Vec<String>,
    pub target_connection_id: String,
    pub target_database: String,
    pub target_schema: Option<String>,
    pub target_tables: Vec<String>,
    pub case_sensitive_identifiers: bool,
    /// Include views in the schema comparison. View definitions are still
    /// outside the sync planner; only their visible schema is compared.
    pub compare_views: bool,
    /// Include functions and procedures in read-only schema comparison.
    pub compare_routines: bool,
    /// Include triggers in read-only schema comparison.
    pub compare_triggers: bool,
    pub compare_indexes: bool,
    pub compare_foreign_keys: bool,
    pub ignore_comments: bool,
    pub ignore_auto_increment: bool,
    pub ignore_charset_collation: bool,
    pub ignore_table_options: bool,
    pub compare_column_order: bool,
    /// User-defined type mapping overrides. When present, these take
    /// precedence over built-in canonical type mappings.
    pub type_mapping_overrides: super::TypeMappingOverrides,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_with_flags(source_truncated: bool, target_truncated: bool) -> DataCompareResult {
        DataCompareResult {
            source_truncated,
            target_truncated,
            ..Default::default()
        }
    }

    #[test]
    fn sync_sql_blocking_only_tracks_truncation_and_dependency_metadata() {
        assert!(!DataCompareBatchResult::default().is_sync_sql_blocked());

        assert!(
            DataCompareBatchResult {
                table_results: vec![result_with_flags(true, false)],
                ..Default::default()
            }
            .is_sync_sql_blocked()
        );
        assert!(
            DataCompareBatchResult {
                table_results: vec![result_with_flags(false, true)],
                ..Default::default()
            }
            .is_sync_sql_blocked()
        );

        for kind in [
            DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable,
            DataCompareBatchWarningKind::TargetTableMetadataUnavailable,
            DataCompareBatchWarningKind::ConsistentSnapshotUnavailable,
        ] {
            assert!(
                DataCompareBatchResult {
                    batch_warnings: vec![DataCompareBatchWarning {
                        table: None,
                        kind,
                        error: "metadata unavailable".to_string(),
                    }],
                    ..Default::default()
                }
                .is_sync_sql_blocked()
            );
        }

        assert!(
            !DataCompareBatchResult {
                table_failures: vec![DataCompareTableFailure {
                    table: "failed_table".to_string(),
                    error: "compare failed".to_string(),
                }],
                ..Default::default()
            }
            .is_sync_sql_blocked()
        );
        assert!(
            !DataCompareBatchResult {
                table_results: vec![DataCompareResult {
                    target_table_missing: true,
                    ..Default::default()
                }],
                ..Default::default()
            }
            .is_sync_sql_blocked()
        );
        assert!(
            DataCompareBatchResult {
                table_results: vec![result_with_flags(true, false)],
                batch_warnings: vec![DataCompareBatchWarning {
                    table: Some("users".to_string()),
                    kind: DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable,
                    error: "metadata unavailable".to_string(),
                }],
                ..Default::default()
            }
            .is_sync_sql_blocked()
        );
    }

    #[test]
    fn missing_target_with_truncated_source_still_blocks_sync_sql() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                target_table_missing: true,
                source_truncated: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(result.has_missing_target_tables());
        assert!(result.has_truncated_tables());
        assert!(result.is_sync_sql_blocked());
    }
}
