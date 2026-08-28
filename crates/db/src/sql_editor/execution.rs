use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use one_core::storage::DatabaseType;

use super::statement_ranges::{SqlDialect, SqlTextRange};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SqlMetadataScope {
    pub connection_id: String,
    pub catalog: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub database_type: DatabaseType,
    pub generation: u64,
}

impl SqlMetadataScope {
    pub fn new(
        connection_id: impl Into<String>,
        database_type: DatabaseType,
        generation: u64,
    ) -> Self {
        Self {
            connection_id: connection_id.into(),
            catalog: None,
            database: None,
            schema: None,
            database_type,
            generation,
        }
    }

    pub fn with_catalog(mut self, catalog: Option<String>) -> Self {
        self.catalog = catalog;
        self
    }

    pub fn with_database(mut self, database: Option<String>) -> Self {
        self.database = database;
        self
    }

    pub fn with_schema(mut self, schema: Option<String>) -> Self {
        self.schema = schema;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlDocumentSnapshot {
    pub revision: u64,
    pub text: Arc<str>,
    pub dialect: SqlDialect,
    pub scope: SqlMetadataScope,
}

impl SqlDocumentSnapshot {
    pub fn new(
        revision: u64,
        text: Arc<str>,
        dialect: SqlDialect,
        scope: SqlMetadataScope,
    ) -> Self {
        Self {
            revision,
            text,
            dialect,
            scope,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlExecutionTarget {
    Selection(SqlTextRange),
    CurrentStatement,
    AllStatements,
    ExactRange(SqlTextRange),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlTransactionMode {
    Auto,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlExecutionRequest {
    pub request_id: u64,
    pub document_revision: u64,
    pub full_sql: Arc<str>,
    pub target: SqlExecutionTarget,
    pub resolved_sql: Arc<str>,
    pub source_range: Option<SqlTextRange>,
    pub statement_index: Option<usize>,
    pub sql_fingerprint: u64,
    pub scope: SqlMetadataScope,
    pub transaction_mode: SqlTransactionMode,
}

impl SqlExecutionRequest {
    pub fn new(
        request_id: u64,
        document: SqlDocumentSnapshot,
        target: SqlExecutionTarget,
        resolved_sql: Arc<str>,
        statement_index: Option<usize>,
        transaction_mode: SqlTransactionMode,
    ) -> Self {
        let source_range = match target {
            SqlExecutionTarget::Selection(range) | SqlExecutionTarget::ExactRange(range) => {
                Some(range)
            }
            _ => None,
        };
        Self {
            request_id,
            document_revision: document.revision,
            full_sql: document.text,
            target,
            resolved_sql: resolved_sql.clone(),
            source_range,
            statement_index,
            sql_fingerprint: sql_fingerprint(&resolved_sql),
            scope: document.scope,
            transaction_mode,
        }
    }

    pub fn result_source(&self) -> SqlExecutionResultSource {
        SqlExecutionResultSource {
            request_id: self.request_id,
            document_revision: self.document_revision,
            source_range: self.source_range,
            sql_fingerprint: self.sql_fingerprint,
            statement_index: self.statement_index,
        }
    }

    pub fn with_source(mut self, range: SqlTextRange, statement_index: Option<usize>) -> Self {
        self.source_range = Some(range);
        self.statement_index = statement_index;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SqlExecutionResultSource {
    pub request_id: u64,
    pub document_revision: u64,
    pub source_range: Option<SqlTextRange>,
    pub sql_fingerprint: u64,
    pub statement_index: Option<usize>,
}

pub fn sql_fingerprint(sql: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    sql.trim().hash(&mut hasher);
    hasher.finish()
}
