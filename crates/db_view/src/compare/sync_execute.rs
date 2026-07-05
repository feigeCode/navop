use std::sync::Arc;

use db::{ExecOptions, GlobalDbState, SqlSource, StreamingProgress};
use gpui::AsyncApp;
use tokio::sync::mpsc;

use crate::compare::{DataCompareParams, SchemaCompareParams};

/// 同步 SQL 的目标执行范围
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareTargetScope {
    pub connection_id: String,
    pub database: String,
    pub schema: Option<String>,
}

impl CompareTargetScope {
    pub fn from_data_params(params: &DataCompareParams) -> Self {
        Self {
            connection_id: params.target_connection_id.clone(),
            database: params.target_database.clone(),
            schema: params.target_schema.clone(),
        }
    }

    pub fn from_schema_params(params: &SchemaCompareParams) -> Self {
        Self {
            connection_id: params.target_connection_id.clone(),
            database: params.target_database.clone(),
            schema: params.target_schema.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareSyncExecutionOptions {
    pub use_transaction: bool,
    pub continue_on_error: bool,
}

impl Default for CompareSyncExecutionOptions {
    fn default() -> Self {
        Self {
            use_transaction: true,
            continue_on_error: false,
        }
    }
}

impl CompareSyncExecutionOptions {
    pub fn to_exec_options(self) -> ExecOptions {
        ExecOptions {
            stop_on_error: !self.continue_on_error,
            transactional: self.use_transaction && !self.continue_on_error,
            max_rows: None,
            streaming: true,
        }
    }
}

/// 执行同步 SQL。调用方应只传入用户确认或默认安全选中的 SQL。
pub fn execute_sync_sql(
    target: CompareTargetScope,
    sql: String,
    db_state: Arc<GlobalDbState>,
    options: CompareSyncExecutionOptions,
    cx: &mut AsyncApp,
) -> anyhow::Result<mpsc::Receiver<StreamingProgress>> {
    if sql.trim().is_empty() {
        anyhow::bail!("No sync SQL to execute");
    }

    db_state.execute_streaming(
        cx,
        target.connection_id,
        SqlSource::Script(sql),
        Some(target.database),
        target.schema,
        Some(options.to_exec_options()),
    )
}

#[cfg(test)]
mod tests {
    use super::CompareSyncExecutionOptions;

    #[test]
    fn compare_sync_execution_options_enable_streaming_script_execution() {
        let options = CompareSyncExecutionOptions {
            use_transaction: false,
            continue_on_error: true,
        }
        .to_exec_options();

        assert!(!options.stop_on_error);
        assert!(!options.transactional);
        assert_eq!(None, options.max_rows);
        assert!(options.streaming);
    }

    #[test]
    fn compare_sync_execution_options_continue_on_error_disables_transaction() {
        let options = CompareSyncExecutionOptions {
            use_transaction: true,
            continue_on_error: true,
        }
        .to_exec_options();

        assert!(!options.stop_on_error);
        assert!(!options.transactional);
        assert_eq!(None, options.max_rows);
        assert!(options.streaming);
    }

    #[test]
    fn compare_sync_execution_options_default_to_current_safe_behavior() {
        let options = CompareSyncExecutionOptions::default().to_exec_options();

        assert!(options.stop_on_error);
        assert!(options.transactional);
        assert_eq!(None, options.max_rows);
        assert!(options.streaming);
    }
}
