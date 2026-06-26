use crate::types::FieldType;
use one_core::storage::DatabaseType;
use serde::{Deserialize, Serialize};
use sqlparser::dialect::{
    ClickHouseDialect, DuckDbDialect, GenericDialect, MsSqlDialect, MySqlDialect, OracleDialect,
    PostgreSqlDialect, SQLiteDialect,
};
use sqlparser::tokenizer::{Location, Token, Tokenizer};
use std::borrow::Cow;
use std::path::PathBuf;

/// SQL 脚本来源
#[derive(Clone, Debug)]
pub enum SqlSource {
    /// 直接的 SQL 脚本字符串
    Script(String),
    /// SQL 文件路径
    File(PathBuf),
}

impl SqlSource {
    pub fn file_size(&self) -> Option<u64> {
        match self {
            SqlSource::Script(s) => Some(s.len() as u64),
            SqlSource::File(path) => std::fs::metadata(path).ok().map(|m| m.len()),
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self, SqlSource::File(_))
    }
}

/// Execution options for SQL script
#[derive(Debug, Clone)]
pub struct ExecOptions {
    /// Whether to stop execution when encountering an error
    pub stop_on_error: bool,
    /// Whether to wrap the entire script in a transaction
    pub transactional: bool,
    /// Maximum number of rows to return for query results
    pub max_rows: Option<usize>,
    /// 是否启用流式执行（逐条解析执行，适合大文件/大脚本）
    /// 默认 false，会先解析所有语句再执行
    pub streaming: bool,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            stop_on_error: true,
            transactional: false,
            max_rows: Some(1000),
            streaming: false,
        }
    }
}

pub(crate) fn apply_query_max_rows(
    db_type: DatabaseType,
    sql: &str,
    max_rows: Option<usize>,
    is_query: bool,
) -> Cow<'_, str> {
    let Some(max_rows) = max_rows else {
        return Cow::Borrowed(sql);
    };
    let Some(tokens) = simple_select_tokens(&db_type, sql) else {
        return Cow::Borrowed(sql);
    };
    if !is_query || has_existing_row_limit(&tokens) {
        return Cow::Borrowed(sql);
    }

    match db_type {
        DatabaseType::MSSQL => apply_mssql_top(sql, max_rows, &tokens),
        DatabaseType::Oracle => {
            append_query_clause(sql, &format!("FETCH FIRST {max_rows} ROWS ONLY"))
        }
        _ => append_query_clause(sql, &format!("LIMIT {max_rows}")),
    }
}

fn apply_mssql_top<'a>(sql: &'a str, max_rows: usize, tokens: &[SqlToken]) -> Cow<'a, str> {
    let Some(index) = mssql_top_insert_index(&tokens) else {
        return Cow::Borrowed(sql);
    };

    let mut rewritten = String::with_capacity(sql.len() + 16);
    rewritten.push_str(&sql[..index]);
    rewritten.push_str(&format!(" TOP ({max_rows})"));
    rewritten.push_str(&sql[index..]);
    Cow::Owned(rewritten)
}

fn append_query_clause<'a>(sql: &'a str, clause: &str) -> Cow<'a, str> {
    let trimmed_end = sql.trim_end();
    let trailing_ws = &sql[trimmed_end.len()..];
    let (body, terminator) = trimmed_end
        .strip_suffix(';')
        .map(|body| (body.trim_end(), ";"))
        .unwrap_or((trimmed_end, ""));
    Cow::Owned(format!("{body} {clause}{terminator}{trailing_ws}"))
}

fn simple_select_tokens(db_type: &DatabaseType, sql: &str) -> Option<Vec<SqlToken>> {
    let tokens = significant_tokens(db_type, sql)?;
    tokens
        .first()
        .is_some_and(|token| token.depth == 0 && word_eq(&token.token, "SELECT"))
        .then_some(tokens)
}

fn has_existing_row_limit(tokens: &[SqlToken]) -> bool {
    tokens.iter().any(|token| {
        token.depth == 0
            && (word_eq(&token.token, "LIMIT")
                || word_eq(&token.token, "FETCH")
                || word_eq(&token.token, "FORMAT")
                || word_eq(&token.token, "ROWNUM")
                || word_eq(&token.token, "TOP"))
    })
}

fn mssql_top_insert_index(tokens: &[SqlToken]) -> Option<usize> {
    let select = tokens
        .iter()
        .position(|token| token.depth == 0 && word_eq(&token.token, "SELECT"))?;
    let mut insert_after = select;
    let next = tokens.get(select + 1);
    if next.is_some_and(|token| word_eq(&token.token, "ALL") || word_eq(&token.token, "DISTINCT")) {
        insert_after = select + 1;
    }
    if tokens
        .get(insert_after + 1)
        .is_some_and(|token| word_eq(&token.token, "TOP"))
    {
        return None;
    }
    Some(tokens[insert_after].end)
}

#[derive(Debug)]
struct SqlToken {
    token: Token,
    depth: usize,
    end: usize,
}

fn significant_tokens(db_type: &DatabaseType, sql: &str) -> Option<Vec<SqlToken>> {
    let dialect = tokenizer_dialect(db_type);
    let mut tokenizer = Tokenizer::new(dialect.as_ref(), sql);
    let tokens = tokenizer.tokenize_with_location().ok()?;
    let mut depth = 0usize;
    let mut output = Vec::new();
    for token in tokens {
        match token.token {
            Token::Whitespace(_) | Token::EOF => {}
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => output.push(SqlToken {
                end: byte_index_for_location(sql, token.span.end),
                token: token.token,
                depth,
            }),
        }
    }
    Some(output)
}

fn tokenizer_dialect(db_type: &DatabaseType) -> Box<dyn sqlparser::dialect::Dialect> {
    match db_type {
        DatabaseType::MySQL => Box::new(MySqlDialect {}),
        DatabaseType::PostgreSQL => Box::new(PostgreSqlDialect {}),
        DatabaseType::SQLite => Box::new(SQLiteDialect {}),
        DatabaseType::DuckDB => Box::new(DuckDbDialect {}),
        DatabaseType::MSSQL => Box::new(MsSqlDialect {}),
        DatabaseType::Oracle => Box::new(OracleDialect {}),
        DatabaseType::ClickHouse => Box::new(ClickHouseDialect {}),
        DatabaseType::External { .. } => Box::new(GenericDialect {}),
    }
}

fn word_eq(token: &Token, expected: &str) -> bool {
    matches!(
        token,
        Token::Word(word)
            if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(expected)
    )
}

fn byte_index_for_location(sql: &str, location: Location) -> usize {
    let (mut line, mut column) = (1, 1);
    for (index, ch) in sql.char_indices() {
        if line == location.line && column == location.column {
            return index;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    sql.len()
}

/// Result of a single SQL statement execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SqlResult {
    /// Query result (SELECT, SHOW, etc.)
    Query(QueryResult),
    /// Execution result (INSERT, UPDATE, DELETE, DDL, etc.)
    Exec(ExecResult),
    /// Error result
    Error(SqlErrorInfo),
}

impl SqlResult {
    pub fn is_error(&self) -> bool {
        matches!(self, SqlResult::Error(_))
    }
}

/// Column metadata for query results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryColumnMeta {
    /// Column name
    pub name: String,
    /// Original database type (e.g., "VARCHAR(255)", "INT")
    pub db_type: String,
    /// Abstract field type for UI rendering
    pub field_type: FieldType,
    /// Whether the column is nullable
    pub nullable: bool,
}

impl QueryColumnMeta {
    pub fn new(name: impl Into<String>, db_type: impl Into<String>) -> Self {
        let db_type_str = db_type.into();
        let field_type = FieldType::from_db_type(&db_type_str);
        Self {
            name: name.into(),
            db_type: db_type_str,
            field_type,
            nullable: true,
        }
    }

    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = nullable;
        self
    }
}

/// Query result with data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Original SQL statement
    pub sql: String,
    /// Column names
    pub columns: Vec<String>,
    /// Column metadata with type information
    pub column_meta: Vec<QueryColumnMeta>,
    /// Row data (each row is a vector of optional strings)
    pub rows: Vec<Vec<Option<String>>>,
    /// Execution time in milliseconds
    #[serde(with = "elapsed_ms_serde")]
    pub elapsed_ms: u128,
}

/// Execution result for non-query statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    /// Original SQL statement
    pub sql: String,
    /// Number of rows affected
    pub rows_affected: u64,
    /// Execution time in milliseconds
    #[serde(with = "elapsed_ms_serde")]
    pub elapsed_ms: u128,
    /// Optional message
    pub message: Option<String>,
}

/// Error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlErrorInfo {
    /// Original SQL statement
    pub sql: String,
    /// Error message
    pub message: String,
}

mod elapsed_ms_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64((*value).try_into().unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(u64::deserialize(deserializer)? as u128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::DatabaseType;

    #[test]
    fn query_max_rows_adds_limit_for_limit_dialects() {
        let sql = apply_query_max_rows(DatabaseType::MySQL, "select * from users", Some(25), true);
        assert_eq!("select * from users LIMIT 25", sql);
    }

    #[test]
    fn query_max_rows_keeps_existing_limit() {
        let sql = apply_query_max_rows(
            DatabaseType::PostgreSQL,
            "select * from users limit 5",
            Some(25),
            true,
        );
        assert_eq!("select * from users limit 5", sql);
    }

    #[test]
    fn query_max_rows_adds_mssql_top() {
        let sql = apply_query_max_rows(
            DatabaseType::MSSQL,
            "select distinct id from users",
            Some(25),
            true,
        );
        assert_eq!("select distinct TOP (25) id from users", sql);
    }

    #[test]
    fn query_max_rows_adds_oracle_fetch() {
        let sql =
            apply_query_max_rows(DatabaseType::Oracle, "select * from users;", Some(25), true);
        assert_eq!("select * from users FETCH FIRST 25 ROWS ONLY;", sql);
    }

    #[test]
    fn query_max_rows_ignores_non_queries_and_unbounded_options() {
        assert_eq!(
            "update users set name = 'a'",
            apply_query_max_rows(
                DatabaseType::MySQL,
                "update users set name = 'a'",
                Some(25),
                false,
            )
        );
        assert_eq!(
            "select * from users",
            apply_query_max_rows(DatabaseType::MySQL, "select * from users", None, true)
        );
        assert_eq!(
            "show tables",
            apply_query_max_rows(DatabaseType::MySQL, "show tables", Some(25), true)
        );
    }

    #[test]
    fn query_max_rows_ignores_limit_inside_string() {
        let sql = apply_query_max_rows(
            DatabaseType::SQLite,
            "select 'limit 1' as text from users",
            Some(25),
            true,
        );
        assert_eq!("select 'limit 1' as text from users LIMIT 25", sql);
    }
}

pub fn format_message(sql: &str, rows_affected: u64) -> String {
    let trimmed = sql.trim().to_uppercase();

    if trimmed.starts_with("INSERT") {
        format!("Inserted {} row(s)", rows_affected)
    } else if trimmed.starts_with("UPDATE") {
        format!("Updated {} row(s)", rows_affected)
    } else if trimmed.starts_with("DELETE") {
        format!("Deleted {} row(s)", rows_affected)
    } else if trimmed.starts_with("REPLACE") {
        format!("Replaced {} row(s)", rows_affected)
    } else if trimmed.starts_with("CREATE") {
        "Object created successfully".to_string()
    } else if trimmed.starts_with("ALTER") {
        "Object altered successfully".to_string()
    } else if trimmed.starts_with("DROP") {
        "Object dropped successfully".to_string()
    } else if trimmed.starts_with("TRUNCATE") {
        "Table truncated successfully".to_string()
    } else if trimmed.starts_with("RENAME") {
        "Object renamed successfully".to_string()
    } else if trimmed.starts_with("USE") {
        "Database changed successfully".to_string()
    } else if trimmed.starts_with("SET") {
        "Variable set successfully".to_string()
    } else if trimmed.starts_with("BEGIN") || trimmed.starts_with("START TRANSACTION") {
        "Transaction started".to_string()
    } else if trimmed.starts_with("COMMIT") {
        "Transaction committed".to_string()
    } else if trimmed.starts_with("ROLLBACK") {
        "Transaction rolled back".to_string()
    } else {
        format!(
            "Query executed successfully, {} row(s) affected",
            rows_affected
        )
    }
}

/// Statement type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementType {
    /// Query statement (SELECT, SHOW, etc.)
    Query,
    /// Data manipulation (INSERT, UPDATE, DELETE)
    Dml,
    /// Data definition (CREATE, ALTER, DROP)
    Ddl,
    /// Transaction control (BEGIN, COMMIT, ROLLBACK)
    Transaction,
    /// Database commands (USE, SET)
    Command,
    /// Other execution statements
    Exec,
}
