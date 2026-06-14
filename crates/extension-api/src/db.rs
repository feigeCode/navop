use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_options_default_is_bounded() {
        let options = ExecOptions::default();

        assert_eq!(Some(1_000), options.max_rows);
        assert_eq!(Some(30_000), options.timeout_ms);
        assert!(!options.stream);
    }

    #[test]
    fn row_batch_can_be_constructed() {
        let batch = RowBatch {
            columns: vec![Column {
                name: "id".to_string(),
                type_name: "int".to_string(),
                nullable: false,
            }],
            rows: vec![vec![DbValue::Integer(1)]],
            next_cursor: None,
        };

        assert_eq!(1, batch.columns.len());
        assert_eq!(1, batch.rows.len());
    }

    #[test]
    fn wit_db_interface_uses_typed_results_and_session_methods() {
        let wit = include_str!("../wit/db.wit");

        assert!(wit.contains("variant db-value"));
        assert!(wit.contains("record db-error"));
        assert!(wit.contains("record row-batch"));
        assert!(wit.contains(
            "open-session: func(connection-id: string, database: option<string>) -> result<session, db-error>;"
        ));
        assert!(wit.contains(
            "execute: func(sql: string, options: exec-options) -> result<row-batch, db-error>;"
        ));
        assert!(!wit.contains("execute: func(session: borrow<session>"));
        assert!(!wit.contains("-> string"));
    }

    #[test]
    fn wit_package_parses() {
        let wit_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");
        let mut resolve = wit_parser::Resolve::new();

        resolve.push_dir(wit_dir).expect("WIT package parses");
    }
}
