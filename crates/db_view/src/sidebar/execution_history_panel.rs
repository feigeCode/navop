use db::SqlResult;
use gpui::{App, Context, FocusHandle, Focusable};
use rust_i18n::t;

use super::execution_history::{ExecutionContext, ExecutionHistory};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ExecutionHistoryFilter {
    #[default]
    All,
    Success,
    Error,
}

pub struct ExecutionHistoryPanel {
    pub(super) history: ExecutionHistory,
    pub(super) filter: ExecutionHistoryFilter,
    focus_handle: FocusHandle,
}

impl ExecutionHistoryPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            history: ExecutionHistory::default(),
            filter: ExecutionHistoryFilter::All,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn record_sql_results(
        &mut self,
        context: ExecutionContext,
        results: &[SqlResult],
        cx: &mut Context<Self>,
    ) {
        for result in results {
            self.history.record_result(context.clone(), result);
        }
        cx.notify();
    }

    pub fn record_table_data_results(
        &mut self,
        context: ExecutionContext,
        sql: String,
        results: &[SqlResult],
        cx: &mut Context<Self>,
    ) {
        self.history.record_results(
            context,
            results,
            Some(sql),
            t!("TableDataGrid.execute_success").to_string(),
            |error| t!("TableDataGrid.execute_failed", error = error).to_string(),
        );
        cx.notify();
    }

    pub fn record_transport_error(
        &mut self,
        context: ExecutionContext,
        sql: String,
        error: String,
        cx: &mut Context<Self>,
    ) {
        self.history.record_transport_error(context, sql, error);
        cx.notify();
    }

    pub(super) fn clear(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        cx.notify();
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
