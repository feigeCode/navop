use serde::{Deserialize, Serialize};

use crate::{SqlAccess, classify_sql};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub database: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOptions {
    pub max_rows: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub stream: bool,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            max_rows: Some(1_000),
            timeout_ms: Some(30_000),
            stream: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DbValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowBatch {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<DbValue>>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbError {
    pub code: String,
    pub message: String,
}

impl DbError {
    pub fn permission_denied(permission: impl AsRef<str>) -> Self {
        Self {
            code: "permission_denied".to_string(),
            message: format!("permission denied: {}", permission.as_ref()),
        }
    }

    pub fn connection_not_found(connection_id: impl AsRef<str>) -> Self {
        Self {
            code: "connection_not_found".to_string(),
            message: format!("connection not found: {}", connection_id.as_ref()),
        }
    }

    pub fn query_failed(message: impl Into<String>) -> Self {
        Self {
            code: "query_failed".to_string(),
            message: message.into(),
        }
    }

    pub fn invalid_resource(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_resource".to_string(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenSessionRequest {
    pub connection_id: String,
    pub database: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteSqlRequest {
    pub session_id: String,
    pub connection_id: String,
    pub sql: String,
    pub options: ExecOptions,
}

impl ExecuteSqlRequest {
    pub fn access(&self) -> SqlAccess {
        classify_sql(&self.sql)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqlAccess;

    #[test]
    fn execute_sql_request_classifies_script_access() {
        let request = ExecuteSqlRequest {
            session_id: "session-1".to_string(),
            connection_id: "conn-1".to_string(),
            sql: "insert into t values (1)".to_string(),
            options: ExecOptions::default(),
        };

        assert_eq!(SqlAccess::Write, request.access());
    }

    #[test]
    fn db_error_has_stable_permission_denied_code() {
        let error = DbError::permission_denied("db:write:conn-1");

        assert_eq!("permission_denied", error.code);
        assert!(error.message.contains("db:write:conn-1"));
    }

    #[test]
    fn db_error_has_stable_invalid_resource_code() {
        let error = DbError::invalid_resource("closed session resource");

        assert_eq!("invalid_resource", error.code);
        assert_eq!("closed session resource", error.message);
    }
}
