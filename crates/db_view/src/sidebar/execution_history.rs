use db::{ExecResult, QueryResult, SqlErrorInfo, SqlResult};
use rust_i18n::t;

pub const MAX_EXECUTION_RECORDS: usize = 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionContext {
    pub connection_id: String,
    pub database: Option<String>,
    pub schema: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionStatus {
    Success,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionRecord {
    pub context: ExecutionContext,
    pub status: ExecutionStatus,
    pub sql: String,
    pub summary: String,
    pub details: Vec<String>,
    pub affected_rows: u64,
    pub returned_rows: Option<usize>,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionHistory {
    records: Vec<ExecutionRecord>,
}

impl ExecutionHistory {
    pub fn records(&self) -> &[ExecutionRecord] {
        &self.records
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    pub fn record_result(&mut self, context: ExecutionContext, result: &SqlResult) {
        let record = ExecutionRecord::from_result(context, result);
        self.push(record);
    }

    pub fn record_results(
        &mut self,
        context: ExecutionContext,
        results: &[SqlResult],
        sql: Option<String>,
        success_summary: impl Into<String>,
        failure_summary: impl Fn(&str) -> String,
    ) {
        let Some(sql) = sql else {
            for result in results {
                self.record_result(context.clone(), result);
            }
            return;
        };
        self.push(ExecutionRecord::from_results(
            context,
            results,
            sql,
            success_summary.into(),
            failure_summary,
        ));
    }

    pub fn record_transport_error(
        &mut self,
        context: ExecutionContext,
        sql: String,
        error: String,
    ) {
        self.push(ExecutionRecord {
            context,
            status: ExecutionStatus::Error,
            sql,
            summary: error.clone(),
            details: vec![error],
            affected_rows: 0,
            returned_rows: None,
            elapsed_ms: 0,
        });
    }

    fn push(&mut self, record: ExecutionRecord) {
        self.records.push(record);
        if self.records.len() > MAX_EXECUTION_RECORDS {
            let overflow = self.records.len() - MAX_EXECUTION_RECORDS;
            self.records.drain(..overflow);
        }
    }
}

impl ExecutionRecord {
    pub fn from_result(context: ExecutionContext, result: &SqlResult) -> Self {
        match result {
            SqlResult::Query(QueryResult {
                sql,
                rows,
                elapsed_ms,
                ..
            }) => Self {
                context,
                status: ExecutionStatus::Success,
                sql: sql.clone(),
                summary: t!("DatabaseSidebar.query_succeeded").to_string(),
                details: Vec::new(),
                affected_rows: 0,
                returned_rows: Some(rows.len()),
                elapsed_ms: *elapsed_ms,
            },
            SqlResult::Exec(ExecResult {
                sql,
                rows_affected,
                elapsed_ms,
                message,
            }) => Self {
                context,
                status: ExecutionStatus::Success,
                sql: sql.clone(),
                summary: message
                    .clone()
                    .filter(|message| !message.trim().is_empty())
                    .unwrap_or_else(|| t!("DatabaseSidebar.execution_succeeded").to_string()),
                details: message.clone().into_iter().collect(),
                affected_rows: *rows_affected,
                returned_rows: None,
                elapsed_ms: *elapsed_ms,
            },
            SqlResult::Error(SqlErrorInfo { sql, message }) => Self {
                context,
                status: ExecutionStatus::Error,
                sql: sql.clone(),
                summary: message.clone(),
                details: vec![message.clone()],
                affected_rows: 0,
                returned_rows: None,
                elapsed_ms: 0,
            },
        }
    }

    pub fn from_results(
        context: ExecutionContext,
        results: &[SqlResult],
        sql: String,
        success_summary: String,
        failure_summary: impl Fn(&str) -> String,
    ) -> Self {
        let mut errors = Vec::new();
        let mut details = Vec::new();
        let mut affected_rows = 0;
        let mut returned_rows = None;
        let mut elapsed_ms = 0;

        for result in results {
            match result {
                SqlResult::Query(result) => {
                    elapsed_ms += result.elapsed_ms;
                    returned_rows = Some(returned_rows.unwrap_or_default() + result.rows.len());
                }
                SqlResult::Exec(result) => {
                    affected_rows += result.rows_affected;
                    elapsed_ms += result.elapsed_ms;
                    if let Some(message) = result
                        .message
                        .as_ref()
                        .filter(|message| !message.trim().is_empty())
                    {
                        details.push(message.clone());
                    }
                }
                SqlResult::Error(error) => errors.push(error.message.clone()),
            }
        }
        let status = if errors.is_empty() {
            ExecutionStatus::Success
        } else {
            details.splice(0..0, errors.clone());
            ExecutionStatus::Error
        };
        let summary = errors
            .first()
            .map(|error| failure_summary(error))
            .unwrap_or(success_summary);

        Self {
            context,
            status,
            sql,
            summary,
            details,
            affected_rows,
            returned_rows,
            elapsed_ms,
        }
    }
}

#[cfg(test)]
#[path = "execution_history_tests.rs"]
mod tests;
