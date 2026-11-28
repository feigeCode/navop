use db::SqlResult;
use gpui::{App, Context, FocusHandle, Focusable};
use one_core::storage::{GlobalStorageState, SqlExecutionHistoryRepository};
use rust_i18n::t;
use std::sync::Arc;

use super::execution_history::{
    ExecutionContext, ExecutionHistory, ExecutionRecord, ExecutionStatus, MAX_EXECUTION_RECORDS,
    recent_sql_statements,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ExecutionHistoryFilter {
    #[default]
    All,
    Success,
    Error,
}

impl ExecutionHistoryFilter {
    /// 记录是否属于当前筛选。
    pub(super) fn matches(self, record: &ExecutionRecord) -> bool {
        match self {
            Self::All => true,
            Self::Success => record.status == ExecutionStatus::Success,
            Self::Error => record.status == ExecutionStatus::Error,
        }
    }
}

pub struct ExecutionHistoryPanel {
    pub(super) history: ExecutionHistory,
    pub(super) filter: ExecutionHistoryFilter,
    connection_ids: Vec<String>,
    repository: Option<Arc<SqlExecutionHistoryRepository>>,
    focus_handle: FocusHandle,
}

impl ExecutionHistoryPanel {
    pub fn new(connection_ids: Vec<String>, cx: &mut Context<Self>) -> Self {
        let repository = cx
            .try_global::<GlobalStorageState>()
            .and_then(|state| state.storage.get::<SqlExecutionHistoryRepository>());
        let history = repository
            .as_ref()
            .and_then(
                |repository| match repository.list(&connection_ids, MAX_EXECUTION_RECORDS) {
                    Ok(mut entries) => {
                        entries.reverse();
                        Some(ExecutionHistory::from_records(
                            entries
                                .into_iter()
                                .map(ExecutionRecord::from_storage_entry)
                                .collect(),
                        ))
                    }
                    Err(error) => {
                        tracing::error!(?error, "failed to load SQL execution history");
                        None
                    }
                },
            )
            .unwrap_or_default();

        Self {
            history,
            filter: ExecutionHistoryFilter::All,
            connection_ids,
            repository,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn record_sql_results(
        &mut self,
        context: ExecutionContext,
        results: &[SqlResult],
        cx: &mut Context<Self>,
    ) {
        let records = results
            .iter()
            .map(|result| self.history.record_result(context.clone(), result))
            .collect::<Vec<_>>();
        self.persist(&records);
        cx.notify();
    }

    pub fn record_table_data_results(
        &mut self,
        context: ExecutionContext,
        sql: String,
        results: &[SqlResult],
        cx: &mut Context<Self>,
    ) {
        let records = self.history.record_results(
            context,
            results,
            Some(sql),
            t!("TableDataGrid.execute_success").to_string(),
            |error| t!("TableDataGrid.execute_failed", error = error).to_string(),
        );
        self.persist(&records);
        cx.notify();
    }

    pub fn record_transport_error(
        &mut self,
        context: ExecutionContext,
        sql: String,
        error: String,
        cx: &mut Context<Self>,
    ) {
        let record = self.history.record_transport_error(context, sql, error);
        self.persist(std::slice::from_ref(&record));
        cx.notify();
    }

    pub(super) fn clear(&mut self, cx: &mut Context<Self>) {
        if let Some(repository) = &self.repository
            && let Err(error) = repository.clear(&self.connection_ids)
        {
            tracing::error!(?error, "failed to clear SQL execution history");
            return;
        }
        self.history.clear();
        cx.notify();
    }

    /// 最近执行过的 SQL（新的在前），给编辑器的 ctrl-h 历史弹层用。
    ///
    /// 只读内存里的记录：`new` 时已从仓库加载过，执行时也会实时追加。
    pub fn recent_sql(&self, limit: usize) -> Vec<String> {
        recent_sql_statements(self.history.records(), limit)
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = &self.repository else {
            return;
        };
        match repository.list(&self.connection_ids, MAX_EXECUTION_RECORDS) {
            Ok(mut entries) => {
                entries.reverse();
                self.history = ExecutionHistory::from_records(
                    entries
                        .into_iter()
                        .map(ExecutionRecord::from_storage_entry)
                        .collect(),
                );
                cx.notify();
            }
            Err(error) => {
                tracing::error!(?error, "failed to reload SQL execution history");
            }
        }
    }

    fn persist(&self, records: &[ExecutionRecord]) {
        let Some(repository) = &self.repository else {
            return;
        };
        for record in records {
            if let Err(error) = repository.record(&record.to_storage_entry()) {
                tracing::error!(?error, "failed to persist SQL execution history");
            }
        }
    }

    pub(super) fn set_filter(&mut self, filter: ExecutionHistoryFilter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            cx.notify();
        }
    }
}

impl Focusable for ExecutionHistoryPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
