use std::sync::Arc;

use db::{ExecOptions, GlobalDbState, SqlResult};
use gpui::AsyncApp;

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

/// 执行同步 SQL。调用方应只传入用户确认或默认安全选中的 SQL。
pub async fn execute_sync_sql(
    target: CompareTargetScope,
    sql: String,
    db_state: Arc<GlobalDbState>,
    cx: &mut AsyncApp,
) -> anyhow::Result<usize> {
    if sql.trim().is_empty() {
        anyhow::bail!("No sync SQL to execute");
    }

    let results = db_state
        .execute_script(
            cx,
            target.connection_id,
            sql,
            Some(target.database),
            target.schema,
            Some(sync_exec_options()),
        )
        .await?;

    if let Some(message) = first_sql_error(&results) {
        anyhow::bail!("{}", message);
    }

    Ok(results.len())
}

fn sync_exec_options() -> ExecOptions {
    ExecOptions {
        stop_on_error: true,
        transactional: true,
        max_rows: None,
        streaming: false,
    }
}

fn first_sql_error(results: &[SqlResult]) -> Option<String> {
    results.iter().find_map(|result| match result {
        SqlResult::Error(error) => Some(error.message.clone()),
        _ => None,
    })
}
