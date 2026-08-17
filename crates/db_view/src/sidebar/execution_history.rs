use db::{ExecResult, QueryResult, SqlErrorInfo, SqlResult};
use one_core::storage::{SqlExecutionHistoryEntry, SqlExecutionStatus, now as storage_now};
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
    pub executed_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionHistory {
    records: Vec<ExecutionRecord>,
}

impl ExecutionHistory {
    pub fn from_records(records: Vec<ExecutionRecord>) -> Self {
        Self { records }
    }

    pub fn records(&self) -> &[ExecutionRecord] {
        &self.records
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    pub fn record_result(
        &mut self,
        context: ExecutionContext,
        result: &SqlResult,
    ) -> ExecutionRecord {
        let record = ExecutionRecord::from_result(context, result);
        self.push(record.clone());
        record
    }

    pub fn record_results(
        &mut self,
        context: ExecutionContext,
        results: &[SqlResult],
        sql: Option<String>,
        success_summary: impl Into<String>,
        failure_summary: impl Fn(&str) -> String,
    ) -> Vec<ExecutionRecord> {
        let Some(sql) = sql else {
            let mut records = Vec::with_capacity(results.len());
            for result in results {
                records.push(self.record_result(context.clone(), result));
            }
            return records;
        };
        let record = ExecutionRecord::from_results(
            context,
            results,
            sql,
            success_summary.into(),
            failure_summary,
        );
        self.push(record.clone());
        vec![record]
    }

    pub fn record_transport_error(
        &mut self,
        context: ExecutionContext,
        sql: String,
        error: String,
    ) -> ExecutionRecord {
        let record = ExecutionRecord {
            context,
            status: ExecutionStatus::Error,
            sql,
            summary: error.clone(),
            details: vec![error],
            affected_rows: 0,
            returned_rows: None,
            elapsed_ms: 0,
            executed_at: storage_now(),
        };
        self.push(record.clone());
        record
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
                executed_at: storage_now(),
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
                executed_at: storage_now(),
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
                executed_at: storage_now(),
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
            executed_at: storage_now(),
        }
    }

    pub fn to_storage_entry(&self) -> SqlExecutionHistoryEntry {
        SqlExecutionHistoryEntry {
            id: None,
            connection_id: self.context.connection_id.clone(),
            database: self.context.database.clone(),
            schema: self.context.schema.clone(),
            status: match self.status {
                ExecutionStatus::Success => SqlExecutionStatus::Success,
                ExecutionStatus::Error => SqlExecutionStatus::Error,
            },
            sql: self.sql.clone(),
            summary: self.summary.clone(),
            details: self.details.clone(),
            affected_rows: self.affected_rows,
            returned_rows: self.returned_rows,
            elapsed_ms: self.elapsed_ms,
            executed_at: self.executed_at,
        }
    }

    pub fn from_storage_entry(entry: SqlExecutionHistoryEntry) -> Self {
        Self {
            context: ExecutionContext {
                connection_id: entry.connection_id,
                database: entry.database,
                schema: entry.schema,
            },
            status: match entry.status {
                SqlExecutionStatus::Success => ExecutionStatus::Success,
                SqlExecutionStatus::Error => ExecutionStatus::Error,
            },
            sql: entry.sql,
            summary: entry.summary,
            details: entry.details,
            affected_rows: entry.affected_rows,
            returned_rows: entry.returned_rows,
            elapsed_ms: entry.elapsed_ms,
            executed_at: entry.executed_at,
        }
    }
}

#[cfg(test)]
#[path = "execution_history_tests.rs"]
mod tests;
